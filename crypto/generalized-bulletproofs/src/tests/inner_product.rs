// The inner product relation is P = sum(g_bold * a, h_bold * b, g * (a * b))

use rand_core::OsRng;

use transcript::{Transcript, RecommendedTranscript};

use ciphersuite::{
  group::{ff::Field, Group},
  Ciphersuite, Ristretto,
};

use crate::{
  ScalarVector, PointVector,
  inner_product::{IpStatement, IpWitness},
  tests::generators,
};

#[test]
fn test_zero_inner_product() {
  let P = <Ristretto as Ciphersuite>::G::identity();

  let generators = generators::<Ristretto>(1);
  let reduced = generators.reduce(1).unwrap();
  let statement = IpStatement::<_, Ristretto>::new(
    reduced,
    ScalarVector(vec![<Ristretto as Ciphersuite>::F::ONE; 1]),
    <Ristretto as Ciphersuite>::F::ONE,
    P,
  )
  .unwrap();
  let witness = IpWitness::<Ristretto>::new(
    ScalarVector::<<Ristretto as Ciphersuite>::F>::new(1),
    ScalarVector::<<Ristretto as Ciphersuite>::F>::new(1),
  )
  .unwrap();

  let mut transcript = RecommendedTranscript::new(b"Zero IP Test");
  let proof = statement.clone().prove(&mut transcript.clone(), witness).unwrap();

  let mut verifier = generators.batch_verifier();
  statement.verify(&mut OsRng, &mut verifier, &mut transcript, proof).unwrap();
  assert!(generators.verify(verifier));
}

#[test]
fn test_inner_product() {
  // P = sum(g_bold * a, h_bold * b)
  let generators = generators::<Ristretto>(32);
  let mut verifier = generators.batch_verifier();
  for i in [1, 2, 4, 8, 16, 32] {
    let generators = generators.reduce(i).unwrap();
    let g = generators.g();
    assert_eq!(generators.len(), i);
    let mut g_bold = vec![];
    let mut h_bold = vec![];
    for i in 0 .. i {
      g_bold.push(generators.g_bold(i));
      h_bold.push(generators.h_bold(i));
    }
    let g_bold = PointVector::<Ristretto>(g_bold);
    let h_bold = PointVector::<Ristretto>(h_bold);

    let mut a = ScalarVector::<<Ristretto as Ciphersuite>::F>::new(i);
    let mut b = ScalarVector::<<Ristretto as Ciphersuite>::F>::new(i);

    for i in 0 .. i {
      a[i] = <Ristretto as Ciphersuite>::F::random(&mut OsRng);
      b[i] = <Ristretto as Ciphersuite>::F::random(&mut OsRng);
    }

    let P = g_bold.multiexp(&a) + h_bold.multiexp(&b) + (g * a.inner_product(&b));

    let statement = IpStatement::<_, Ristretto>::new(
      generators,
      ScalarVector(vec![<Ristretto as Ciphersuite>::F::ONE; i]),
      <Ristretto as Ciphersuite>::F::ONE,
      P,
    )
    .unwrap();
    let witness = IpWitness::<Ristretto>::new(a, b).unwrap();

    let mut transcript = RecommendedTranscript::new(b"IP Test");
    let proof = statement.clone().prove(&mut transcript.clone(), witness).unwrap();
    statement.verify(&mut OsRng, &mut verifier, &mut transcript, proof).unwrap();
  }
  assert!(generators.verify(verifier));
}
