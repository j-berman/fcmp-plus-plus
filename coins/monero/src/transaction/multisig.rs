use std::{rc::Rc, cell::RefCell};

use rand_core::{RngCore, CryptoRng, SeedableRng};
use rand_chacha::ChaCha12Rng;

use curve25519_dalek::{scalar::Scalar, edwards::{EdwardsPoint, CompressedEdwardsY}};

use monero::{
  Hash, VarInt,
  consensus::{Encodable, deserialize},
  util::ringct::Key,
  blockdata::transaction::{KeyImage, TxIn, Transaction}
};

use transcript::Transcript as TranscriptTrait;
use frost::{FrostError, MultisigKeys, MultisigParams, sign::{State, StateMachine, AlgorithmMachine}};

use crate::{
  frost::{Transcript, Ed25519},
  random_scalar, key_image, bulletproofs, clsag,
  rpc::Rpc,
  transaction::{TransactionError, SignableTransaction, decoys::{self, Decoys}}
};

pub struct TransactionMachine {
  leader: bool,
  signable: SignableTransaction,
  transcript: Transcript,

  decoys: Vec<Decoys>,

  our_images: Vec<EdwardsPoint>,
  output_masks: Option<Scalar>,
  inputs: Vec<Rc<RefCell<Option<clsag::Details>>>>,
  msg: Rc<RefCell<Option<[u8; 32]>>>,
  clsags: Vec<AlgorithmMachine<Ed25519, clsag::Multisig>>,

  tx: Option<Transaction>
}

impl SignableTransaction {
  pub async fn multisig<R: RngCore + CryptoRng>(
    mut self,
    label: Vec<u8>,
    rng: &mut R,
    rpc: &Rpc,
    height: usize,
    keys: Rc<MultisigKeys<Ed25519>>,
    included: &[usize]
  ) -> Result<TransactionMachine, TransactionError> {
    let mut our_images = vec![];
    let mut inputs = vec![];
    inputs.resize(self.inputs.len(), Rc::new(RefCell::new(None)));
    let msg = Rc::new(RefCell::new(None));
    let mut clsags = vec![];

    // Create a RNG out of the input shared keys, which either requires the view key or being every
    // sender, and the payments (address and amount), which a passive adversary may be able to know
    // depending on how these transactions are coordinated

    let mut transcript = Transcript::new(label);
    // Also include the spend_key as below only the key offset is included, so this confirms the sum product
    // Useful as confirming the sum product confirms the key image, further guaranteeing the one time
    // properties noted below
    transcript.append_message(b"spend_key", &keys.group_key().0.compress().to_bytes());
    for input in &self.inputs {
      // These outputs can only be spent once. Therefore, it forces all RNGs derived from this
      // transcript (such as the one used to create one time keys) to be unique
      transcript.append_message(b"input_hash", &input.tx.0);
      transcript.append_message(b"input_output_index", &u16::try_from(input.o).unwrap().to_le_bytes());
      // Not including this, with a doxxed list of payments, would allow brute forcing the inputs
      // to determine RNG seeds and therefore the true spends
      transcript.append_message(b"input_shared_key", &input.key_offset.to_bytes());
    }
    for payment in &self.payments {
      transcript.append_message(b"payment_address", &payment.0.as_bytes());
      transcript.append_message(b"payment_amount", &payment.1.to_le_bytes());
    }
    transcript.append_message(b"change", &self.change.as_bytes());

    // Select decoys
    // Ideally, this would be done post entropy, instead of now, yet doing so would require sign
    // to be async which isn't feasible. This should be suitably competent though
    // While this inability means we can immediately create the input, moving it out of the
    // Rc RefCell, keeping it within an Rc RefCell keeps our options flexible
    let decoys = decoys::select(
      &mut ChaCha12Rng::from_seed(transcript.rng_seed(b"decoys", None)),
      rpc,
      height,
      &self.inputs
    ).await.map_err(|e| TransactionError::RpcError(e))?;

    for (i, input) in self.inputs.iter().enumerate() {
      let keys = keys.offset(dalek_ff_group::Scalar(input.key_offset));
      let (image, _) = key_image::generate_share(
        rng,
        &keys.view(included).map_err(|e| TransactionError::FrostError(e))?
      );
      our_images.push(image);

      clsags.push(
        AlgorithmMachine::new(
          clsag::Multisig::new(
            transcript.clone(),
            inputs[i].clone(),
            msg.clone()
          ).map_err(|e| TransactionError::MultisigError(e))?,
          Rc::new(keys),
          included
        ).map_err(|e| TransactionError::FrostError(e))?
      );
    }

    // Verify these outputs by a dummy prep
    self.prepare_outputs(rng)?;

    Ok(TransactionMachine {
      leader: keys.params().i() == included[0],
      signable: self,
      transcript,

      decoys,

      our_images,
      output_masks: None,
      inputs,
      msg,
      clsags,

      tx: None
    })
  }
}

impl StateMachine for TransactionMachine {
  type Signature = Transaction;

  fn preprocess<R: RngCore + CryptoRng>(
    &mut self,
    rng: &mut R
  ) -> Result<Vec<u8>, FrostError> {
    if self.state() != State::Fresh {
      Err(FrostError::InvalidSignTransition(State::Fresh, self.state()))?;
    }

    // Iterate over each CLSAG calling preprocess
    let mut serialized = vec![];
    for clsag in self.clsags.iter_mut() {
      serialized.extend(&clsag.preprocess(rng)?);
    }

    if self.leader {
      let mut entropy = [0; 32];
      rng.fill_bytes(&mut entropy);
      serialized.extend(&entropy);

      let mut rng = ChaCha12Rng::from_seed(self.transcript.rng_seed(b"tx_keys", Some(entropy)));
      // Safe to unwrap thanks to the dummy prepare
      let (commitments, output_masks) = self.signable.prepare_outputs(&mut rng).unwrap();
      self.output_masks = Some(output_masks);

      let bp = bulletproofs::generate(&commitments).unwrap();
      bp.consensus_encode(&mut serialized).unwrap();

      let tx = self.signable.prepare_transaction(&commitments, bp);
      self.tx = Some(tx);
    }

    Ok(serialized)
  }

  fn sign(
    &mut self,
    commitments: &[Option<Vec<u8>>],
    _: &[u8]
  ) -> Result<Vec<u8>, FrostError> {
    if self.state() != State::Preprocessed {
      Err(FrostError::InvalidSignTransition(State::Preprocessed, self.state()))?;
    }

    // FROST commitments, image, commitments, and their proofs
    let clsag_len = 64 + clsag::Multisig::serialized_len();
    let clsag_lens = clsag_len * self.clsags.len();

    // Split out the prep and update the TX
    let mut tx;
    if self.leader {
      tx = self.tx.take().unwrap();
    } else {
      let (l, prep) = commitments.iter().enumerate().filter(|(_, prep)| prep.is_some()).next()
        .ok_or(FrostError::InternalError("no participants".to_string()))?;
      let prep = prep.as_ref().unwrap();

      // Not invalid outputs due to doing a dummy prep as leader
      let (commitments, output_masks) = self.signable.prepare_outputs(
        &mut ChaCha12Rng::from_seed(
          self.transcript.rng_seed(
            b"tx_keys",
            Some(prep[clsag_lens .. (clsag_lens + 32)].try_into().map_err(|_| FrostError::InvalidShare(l))?)
          )
        )
      ).map_err(|_| FrostError::InvalidShare(l))?;
      self.output_masks.replace(output_masks);

      // Verify the provided bulletproofs if not leader
      let bp = deserialize(&prep[(clsag_lens + 32) .. prep.len()]).map_err(|_| FrostError::InvalidShare(l))?;
      if !bulletproofs::verify(&bp, &commitments.iter().map(|c| c.calculate()).collect::<Vec<EdwardsPoint>>()) {
        Err(FrostError::InvalidShare(l))?;
      }

      tx = self.signable.prepare_transaction(&commitments, bp);
    }

    let mut rng = ChaCha12Rng::from_seed(self.transcript.rng_seed(b"pseudo_out_masks", None));
    let mut sum_pseudo_outs = Scalar::zero();
    for c in 0 .. self.clsags.len() {
      // Calculate the key images in order to update the TX
      // Multisig will parse/calculate/validate this as needed, yet doing so here as well provides
      // the easiest API overall
      let mut image = self.our_images[c];
      for (l, serialized) in commitments.iter().enumerate().filter(|(_, s)| s.is_some()) {
        image += CompressedEdwardsY(
          serialized.as_ref().unwrap()[((c * clsag_len) + 64) .. ((c * clsag_len) + 96)]
            .try_into().map_err(|_| FrostError::InvalidCommitment(l))?
        ).decompress().ok_or(FrostError::InvalidCommitment(l))?;
      }

      // TODO sort inputs

      let mut mask = random_scalar(&mut rng);
      if c == (self.clsags.len() - 1) {
        mask = self.output_masks.unwrap() - sum_pseudo_outs;
      } else {
        sum_pseudo_outs += mask;
      }

      self.inputs[c].replace(
        Some(
          clsag::Details::new(
            clsag::Input::new(
              self.signable.inputs[c].commitment,
              self.decoys[c].clone()
            ).map_err(|_| panic!("Signing an input which isn't present in the ring we created for it"))?,
            mask
          )
        )
      );

      tx.prefix.inputs.push(
        TxIn::ToKey {
          amount: VarInt(0),
          key_offsets: self.decoys[c].offsets.clone(),
          k_image: KeyImage { image: Hash(image.compress().to_bytes()) }
        }
      );
    }

    self.msg.replace(Some(tx.signature_hash().unwrap().0));
    self.tx = Some(tx);

    // Iterate over each CLSAG calling sign
    let mut serialized = Vec::with_capacity(self.clsags.len() * 32);
    for (c, clsag) in self.clsags.iter_mut().enumerate() {
      serialized.extend(&clsag.sign(
        &commitments.iter().map(
          |commitments| commitments.clone().map(
            |commitments| commitments[(c * clsag_len) .. ((c * clsag_len) + clsag_len)].to_vec()
          )
        ).collect::<Vec<_>>(),
        &vec![]
      )?);
    }

    Ok(serialized)
  }

  fn complete(&mut self, shares: &[Option<Vec<u8>>]) -> Result<Transaction, FrostError> {
    if self.state() != State::Signed {
      Err(FrostError::InvalidSignTransition(State::Signed, self.state()))?;
    }

    let mut tx = self.tx.take().unwrap();
    let mut prunable = tx.rct_signatures.p.unwrap();
    for (c, clsag) in self.clsags.iter_mut().enumerate() {
      let (clsag, pseudo_out) = clsag.complete(&shares.iter().map(
        |share| share.clone().map(|share| share[(c * 32) .. ((c * 32) + 32)].to_vec())
      ).collect::<Vec<_>>())?;
      prunable.Clsags.push(clsag);
      prunable.pseudo_outs.push(Key { key: pseudo_out.compress().to_bytes() });
    }
    tx.rct_signatures.p = Some(prunable);

    Ok(tx)
  }

  fn multisig_params(&self) -> MultisigParams {
    self.clsags[0].multisig_params()
  }

  fn state(&self) -> State {
    self.clsags[0].state()
  }
}