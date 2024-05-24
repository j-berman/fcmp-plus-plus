use transcript::Transcript;

use multiexp::multiexp_vartime;
use ciphersuite::Ciphersuite;

use generalized_bulletproofs::Generators;

/// Add children to an existing hash.
///
/// For a new hash, pass the hash initialization point as the existing hash.
pub fn hash_grow<T: Transcript, C: Ciphersuite>(
  generators: &Generators<T, C>,
  existing_hash: C::G,
  offset: usize,
  first_child_after_offset: C::F,
  new_children: &[C::F],
) -> Option<C::G> {
  if new_children.is_empty() {
    return None;
  }

  let mut pairs = Vec::with_capacity(new_children.len());
  let mut new_children = new_children.iter().enumerate();
  let (_, first_new) = new_children.next().unwrap();
  pairs.push((*first_new - first_child_after_offset, *generators.g_bold_slice().get(offset)?));
  for (i, new) in new_children {
    pairs.push((*new, *generators.g_bold_slice().get(offset + i)?));
  }
  Some(existing_hash + multiexp_vartime(&pairs))
}

/// Remove children from an existing hash.
///
/// This should only be called when the amount of children removed is less than the amount of
/// children remaining. If less children remain, calling `hash_grow` on a new hash with the
/// remaining children will be faster.
pub fn hash_trim<T: Transcript, C: Ciphersuite>(
  generators: &Generators<T, C>,
  existing_hash: C::G,
  offset: usize,
  children: &[C::F],
) -> Option<C::G> {
  let mut pairs = Vec::with_capacity(children.len());
  for (i, child) in children.iter().enumerate() {
    pairs.push((*child, *generators.g_bold_slice().get(offset + i)?));
  }
  Some(existing_hash - multiexp_vartime(&pairs))
}
