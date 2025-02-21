# Full-Chain Membership Proofs

An implementation of the membership proof from
[FCMP++](https://github.com/kayabaNerve/fcmp-ringct/blob/develop/fcmp%2B%2B.pdf),
which inputs a re-randomized key, re-randomized linking tag generator, a
Pedersen commitment to the linking tag generator, and an amount commitment
before verifying there is a known de-re-randomization for a member of a Merkle
tree.

## Flamegraphs

The flamegraphs were generated with the commands below.

1. [Verify](./flamegraph_verify.svg)

```
CARGO_PROFILE_RELEASE_DEBUG=true cargo +1.84.1 flamegraph --release --unit-test --package full-chain-membership-proofs -- tests::flamegraph_verify --exact --nocapture
```

2. [Prove](./flamegraph_prove.svg)

```
CARGO_PROFILE_RELEASE_DEBUG=true cargo +1.84.1 flamegraph --release --unit-test --package full-chain-membership-proofs -- tests::flamegraph_prove --exact --nocapture
```

3. [`hash_grow`](./flamegraph_hash_grow.svg)

```
CARGO_PROFILE_RELEASE_DEBUG=true cargo +1.84.1 flamegraph --release --unit-test --package full-chain-membership-proofs -- tests::hash_grow --exact --nocapture
```
