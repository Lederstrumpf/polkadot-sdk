// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Primitives for BEEFY protocol.
//!
//! The crate contains shared data types used by BEEFY protocol and documentation (in a form of
//! code) for building a BEEFY light client.
//!
//! BEEFY is a gadget that runs alongside another finality gadget (for instance GRANDPA).
//! For simplicity (and the initially intended use case) the documentation says GRANDPA in places
//! where a more abstract "Finality Gadget" term could be used, but there is no reason why BEEFY
//! wouldn't run with some other finality scheme.
//! BEEFY validator set is supposed to be tracking the Finality Gadget validator set, but note that
//! it will use a different set of keys. For Polkadot use case we plan to use `secp256k1` for BEEFY,
//! while GRANDPA uses `ed25519`.

extern crate alloc;

mod commitment;
mod payload;

pub mod mmr;
pub mod witness;

/// Test utilities
#[cfg(feature = "std")]
pub mod test_utils;

pub use commitment::{Commitment, KnownSignature, SignedCommitment, VersionedFinalityProof};
pub use payload::{known_payloads, BeefyPayloadId, Payload, PayloadProvider};
use sp_mmr_primitives::{
	mmr_lib,
	utils::{self, AncestryHasher},
	AncestryProof,
};

use alloc::{vec, vec::Vec};
use codec::{Codec, Decode, Encode};
use core::fmt::{Debug, Display};
use scale_info::TypeInfo;
use sp_application_crypto::{AppCrypto, AppPublic, ByteArray, RuntimeAppPublic};
use sp_core::H256;
use sp_runtime::traits::{Hash as HashT, HashOutput, Header as HeaderT, Keccak256, NumberFor};

/// Key type for BEEFY module.
pub const KEY_TYPE: sp_core::crypto::KeyTypeId = sp_application_crypto::key_types::BEEFY;

/// Trait representing BEEFY authority id, including custom signature verification.
///
/// Accepts custom hashing fn for the message and custom convertor fn for the signer.
pub trait BeefyAuthorityId<MsgHash: HashT>: RuntimeAppPublic {
	/// Verify a signature.
	///
	/// Return `true` if signature over `msg` is valid for this id.
	fn verify(&self, signature: &<Self as RuntimeAppPublic>::Signature, msg: &[u8]) -> bool;
}

/// Hasher used for BEEFY signatures.
pub type BeefySignatureHasher = sp_runtime::traits::Keccak256;

/// A trait bound which lists all traits which are required to be implemented by
/// a BEEFY AuthorityId type in order to be able to be used in BEEFY Keystore
pub trait AuthorityIdBound:
	Codec
	+ Debug
	+ Clone
	+ AsRef<[u8]>
	+ ByteArray
	+ AppPublic
	+ AppCrypto
	+ RuntimeAppPublic
	+ Display
	+ BeefyAuthorityId<BeefySignatureHasher>
{
}

/// BEEFY cryptographic types for ECDSA crypto
///
/// This module basically introduces four crypto types:
/// - `ecdsa_crypto::Pair`
/// - `ecdsa_crypto::Public`
/// - `ecdsa_crypto::Signature`
/// - `ecdsa_crypto::AuthorityId`
///
/// Your code should use the above types as concrete types for all crypto related
/// functionality.
pub mod ecdsa_crypto {
	use super::{AuthorityIdBound, BeefyAuthorityId, HashT, RuntimeAppPublic, KEY_TYPE};
	use sp_application_crypto::{app_crypto, ecdsa};
	use sp_core::crypto::Wraps;

	app_crypto!(ecdsa, KEY_TYPE);

	/// Identity of a BEEFY authority using ECDSA as its crypto.
	pub type AuthorityId = Public;

	/// Signature for a BEEFY authority using ECDSA as its crypto.
	pub type AuthoritySignature = Signature;

	impl<MsgHash: HashT> BeefyAuthorityId<MsgHash> for AuthorityId
	where
		<MsgHash as HashT>::Output: Into<[u8; 32]>,
	{
		fn verify(&self, signature: &<Self as RuntimeAppPublic>::Signature, msg: &[u8]) -> bool {
			let msg_hash = <MsgHash as HashT>::hash(msg).into();
			match sp_io::crypto::secp256k1_ecdsa_recover_compressed(
				signature.as_inner_ref().as_ref(),
				&msg_hash,
			) {
				Ok(raw_pubkey) => raw_pubkey.as_ref() == AsRef::<[u8]>::as_ref(self),
				_ => false,
			}
		}
	}
	impl AuthorityIdBound for AuthorityId {}
}

/// BEEFY cryptographic types for BLS crypto
///
/// This module basically introduces four crypto types:
/// - `bls_crypto::Pair`
/// - `bls_crypto::Public`
/// - `bls_crypto::Signature`
/// - `bls_crypto::AuthorityId`
///
/// Your code should use the above types as concrete types for all crypto related
/// functionality.

#[cfg(feature = "bls-experimental")]
pub mod bls_crypto {
	use super::{AuthorityIdBound, BeefyAuthorityId, HashT, RuntimeAppPublic, KEY_TYPE};
	use sp_application_crypto::{app_crypto, bls377};
	use sp_core::{bls377::Pair as BlsPair, crypto::Wraps, Pair as _};

	app_crypto!(bls377, KEY_TYPE);

	/// Identity of a BEEFY authority using BLS as its crypto.
	pub type AuthorityId = Public;

	/// Signature for a BEEFY authority using BLS as its crypto.
	pub type AuthoritySignature = Signature;

	impl<MsgHash: HashT> BeefyAuthorityId<MsgHash> for AuthorityId
	where
		<MsgHash as HashT>::Output: Into<[u8; 32]>,
	{
		fn verify(&self, signature: &<Self as RuntimeAppPublic>::Signature, msg: &[u8]) -> bool {
			// `w3f-bls` library uses IETF hashing standard and as such does not expose
			// a choice of hash-to-field function.
			// We are directly calling into the library to avoid introducing new host call.
			// and because BeefyAuthorityId::verify is being called in the runtime so we don't have

			BlsPair::verify(signature.as_inner_ref(), msg, self.as_inner_ref())
		}
	}
	impl AuthorityIdBound for AuthorityId {}
}

/// BEEFY cryptographic types for (ECDSA,BLS) crypto pair
///
/// This module basically introduces four crypto types:
/// - `ecdsa_bls_crypto::Pair`
/// - `ecdsa_bls_crypto::Public`
/// - `ecdsa_bls_crypto::Signature`
/// - `ecdsa_bls_crypto::AuthorityId`
///
/// Your code should use the above types as concrete types for all crypto related
/// functionality.
#[cfg(feature = "bls-experimental")]
pub mod ecdsa_bls_crypto {
	use super::{AuthorityIdBound, BeefyAuthorityId, HashT, RuntimeAppPublic, KEY_TYPE};
	use sp_application_crypto::{app_crypto, ecdsa_bls377};
	use sp_core::{crypto::Wraps, ecdsa_bls377::Pair as EcdsaBlsPair};

	app_crypto!(ecdsa_bls377, KEY_TYPE);

	/// Identity of a BEEFY authority using (ECDSA,BLS) as its crypto.
	pub type AuthorityId = Public;

	/// Signature for a BEEFY authority using (ECDSA,BLS) as its crypto.
	pub type AuthoritySignature = Signature;

	impl<H> BeefyAuthorityId<H> for AuthorityId
	where
		H: HashT,
		H::Output: Into<[u8; 32]>,
	{
		fn verify(&self, signature: &<Self as RuntimeAppPublic>::Signature, msg: &[u8]) -> bool {
			// We can not simply call
			// `EcdsaBlsPair::verify(signature.as_inner_ref(), msg, self.as_inner_ref())`
			// because that invokes ECDSA default verification which performs Blake2b hash
			// which we don't want. This is because ECDSA signatures are meant to be verified
			// on Ethereum network where Keccak hasher is significantly cheaper than Blake2b.
			// See Figure 3 of [OnSc21](https://www.scitepress.org/Papers/2021/106066/106066.pdf)
			// for comparison.
			EcdsaBlsPair::verify_with_hasher::<H>(
				signature.as_inner_ref(),
				msg,
				self.as_inner_ref(),
			)
		}
	}

	impl AuthorityIdBound for AuthorityId {}
}

/// The `ConsensusEngineId` of BEEFY.
pub const BEEFY_ENGINE_ID: sp_runtime::ConsensusEngineId = *b"BEEF";

/// Authority set id starts with zero at BEEFY pallet genesis.
pub const GENESIS_AUTHORITY_SET_ID: u64 = 0;

/// A typedef for validator set id.
pub type ValidatorSetId = u64;

/// A set of BEEFY authorities, a.k.a. validators.
#[derive(Decode, Encode, Debug, PartialEq, Clone, TypeInfo)]
pub struct ValidatorSet<AuthorityId> {
	/// Public keys of the validator set elements
	validators: Vec<AuthorityId>,
	/// Identifier of the validator set
	id: ValidatorSetId,
}

impl<AuthorityId> ValidatorSet<AuthorityId> {
	/// Return a validator set with the given validators and set id.
	pub fn new<I>(validators: I, id: ValidatorSetId) -> Option<Self>
	where
		I: IntoIterator<Item = AuthorityId>,
	{
		let validators: Vec<AuthorityId> = validators.into_iter().collect();
		if validators.is_empty() {
			// No validators; the set would be empty.
			None
		} else {
			Some(Self { validators, id })
		}
	}

	/// Return a reference to the vec of validators.
	pub fn validators(&self) -> &[AuthorityId] {
		&self.validators
	}

	/// Return the validator set id.
	pub fn id(&self) -> ValidatorSetId {
		self.id
	}

	/// Return the number of validators in the set.
	pub fn len(&self) -> usize {
		self.validators.len()
	}
}

/// The index of an authority.
pub type AuthorityIndex = u32;

/// The Hashing used within MMR.
pub type MmrHashing = Keccak256;
/// The type used to represent an MMR root hash.
pub type MmrRootHash = H256;

/// A consensus log item for BEEFY.
#[derive(Decode, Encode, TypeInfo)]
pub enum ConsensusLog<AuthorityId: Codec> {
	/// The authorities have changed.
	#[codec(index = 1)]
	AuthoritiesChange(ValidatorSet<AuthorityId>),
	/// Disable the authority with given index.
	#[codec(index = 2)]
	OnDisabled(AuthorityIndex),
	/// MMR root hash.
	#[codec(index = 3)]
	MmrRoot(MmrRootHash),
}

/// BEEFY vote message.
///
/// A vote message is a direct vote created by a BEEFY node on every voting round
/// and is gossiped to its peers.
// TODO: Remove `Signature` generic type, instead get it from `Id::Signature`.
#[derive(Clone, Debug, Decode, Encode, PartialEq, TypeInfo)]
pub struct VoteMessage<Number, Id, Signature> {
	/// Commit to information extracted from a finalized block
	pub commitment: Commitment<Number>,
	/// Node authority id
	pub id: Id,
	/// Node signature
	pub signature: Signature,
}

/// Trait containing generic methods for BEEFY equivocation proofs.
pub trait BeefyEquivocationProof<Id, Number> {
	/// Returns the authority ids of the misbehaving voters.
	fn offender_ids(&self) -> Vec<&Id>;

	/// Returns the round number at which the infringement occurred.
	fn round_number(&self) -> &Number;

	/// Returns the set id at which the infringement occurred.
	fn set_id(&self) -> ValidatorSetId;
}

/// Proof of voter misbehavior on a given set id. Misbehavior/equivocation in
/// BEEFY happens when a voter votes on the same round/block for different payloads.
/// Proving is achieved by collecting the signed commitments of conflicting votes.
#[derive(Clone, Debug, Decode, Encode, PartialEq, TypeInfo)]
pub struct DoubleVotingProof<Number, Id, Signature> {
	/// The first vote in the equivocation.
	pub first: VoteMessage<Number, Id, Signature>,
	/// The second vote in the equivocation.
	pub second: VoteMessage<Number, Id, Signature>,
}

impl<Number, Id, Signature> DoubleVotingProof<Number, Id, Signature> {
	/// Returns the authority id of the equivocator.
	pub fn offender_id(&self) -> &Id {
		&self.first.id
	}
}

impl<Number, Id, Signature> BeefyEquivocationProof<Id, Number>
	for DoubleVotingProof<Number, Id, Signature>
{
	fn offender_ids(&self) -> Vec<&Id> {
		vec![self.offender_id()]
	}

	fn round_number(&self) -> &Number {
		&self.first.commitment.block_number
	}

	fn set_id(&self) -> ValidatorSetId {
		self.first.commitment.validator_set_id
	}
}

/// Proof of authority misbehavior on a given set id.
/// This proof shows commitment signed on a different fork.
/// See [`CheckForkEquivocationProof`] for proof validity conditions.
#[derive(Clone, Debug, Decode, Encode, PartialEq, TypeInfo)]
pub struct ForkEquivocationProof<Id: RuntimeAppPublic, Header: HeaderT, Hash> {
	/// Commitment for a block on a different fork than one at the same height in
	/// the chain where this proof is submitted.
	pub commitment: Commitment<Header::Number>,
	/// Signatures on this block
	pub signatures: Vec<KnownSignature<Id, Id::Signature>>,
	/// Canonical header at the same height as `commitment.block_number`.
	pub canonical_header: Option<Header>,
	/// Ancestry proof showing that the current best mmr root descends from another mmr root at
	/// `commitment.block_number` than commitment.payload
	pub ancestry_proof: Option<AncestryProof<Hash>>,
}

impl<Id: RuntimeAppPublic, Header: HeaderT, Hash: HashOutput>
	ForkEquivocationProof<Id, Header, Hash>
{
	fn check_fork<NodeHash: HashT<Output = Hash>>(
		&self,
		best_root: Hash,
		mmr_size: u64,
		canonical_header_hash: &Header::Hash,
		first_mmr_block_num: Header::Number,
		best_block_num: Header::Number,
	) -> bool {
		if self.commitment.block_number <= best_block_num {
			if let Some(canonical_header) = &self.canonical_header {
				if check_header_proof(&self.commitment, canonical_header, canonical_header_hash) {
					// avoid verifying the ancestry proof if a valid header proof has been provided
					return true;
				}
			}

			if let Some(ancestry_proof) = &self.ancestry_proof {
				return check_ancestry_proof::<Header, NodeHash>(
					&self.commitment,
					ancestry_proof,
					first_mmr_block_num,
					best_root,
					mmr_size,
				);
			}

			return false;
		}

		true
	}

	/// Validates [ForkEquivocationProof] with the following checks:
	/// - if the commitment is to a block in our history, then at least a header or an ancestry
	///   proof is provided:
	///   - the proof is correct if `self.canonical_header` hashes to `canonical_header_hash`, is at
	///     height `commitment.block_number`, and `commitment.payload` !=
	///     `canonical_payload(canonical_header)`
	///   - the proof is correct if the provided `ancestry_proof` proves
	///   `mmr_root(commitment.block_number) != mmr_root(commitment.payload)`
	/// - `commitment` is signed by all claimed signatories
	///
	/// NOTE: GRANDPA finalization proof is not checked, which leads to slashing on forks. This is
	/// fine since honest validators will not be slashed on the chain finalized by GRANDPA, which is
	/// the only chain that ultimately matters. The only material difference not checking GRANDPA
	/// proofs makes is that validators are not slashed for signing BEEFY commitments prior to the
	/// blocks committed to being finalized by GRANDPA. This is fine too, since the slashing risk of
	/// committing to an incorrect block implies validators will only sign blocks they *know* will
	/// be finalized by GRANDPA.
	pub fn check<MsgHash: HashT, NodeHash: HashT<Output = Hash>>(
		&self,
		// The MMR root of the best block of the chain where this proof is submitted.
		best_root: Hash,
		// The size of the MMR at the best block.
		mmr_size: u64,
		// The hash of the canonical header at the height of `commitment.block_number`.
		canonical_header_hash: &Header::Hash,
		// The block number at which the mmr pallet was added to the runtime.
		first_mmr_block_num: Header::Number,
		// The best block number of the chain where this proof is submitted.
		best_block_num: Header::Number,
	) -> bool
	where
		Id: BeefyAuthorityId<MsgHash> + PartialEq,
	{
		if !self.check_fork::<NodeHash>(
			best_root,
			mmr_size,
			canonical_header_hash,
			first_mmr_block_num,
			best_block_num,
		) {
			return false;
		}

		return self.signatures.iter().all(|signature| {
			// TODO: refactor check_commitment_signature to take a slice of signatories
			check_commitment_signature(
				&self.commitment,
				&signature.validator_id,
				&signature.signature,
			)
		})
	}
}

impl<Id: RuntimeAppPublic, Header: HeaderT, Hash: HashOutput>
	BeefyEquivocationProof<Id, Header::Number> for ForkEquivocationProof<Id, Header, Hash>
{
	fn offender_ids(&self) -> Vec<&Id> {
		self.signatures.iter().map(|signature| &signature.validator_id).collect()
	}

	fn round_number(&self) -> &Header::Number {
		&self.commitment.block_number
	}

	fn set_id(&self) -> ValidatorSetId {
		self.commitment.validator_set_id
	}
}

/// Check a commitment signature by encoding the commitment and
/// verifying the provided signature using the expected authority id.
pub fn check_commitment_signature<Number, Id, MsgHash>(
	commitment: &Commitment<Number>,
	authority_id: &Id,
	signature: &<Id as RuntimeAppPublic>::Signature,
) -> bool
where
	Id: BeefyAuthorityId<MsgHash>,
	Number: Clone + Encode + PartialEq,
	MsgHash: HashT,
{
	let encoded_commitment = commitment.encode();
	BeefyAuthorityId::<MsgHash>::verify(authority_id, signature, &encoded_commitment)
}

/// Verifies the vote equivocation proof by making sure that both votes target
/// different blocks and that its signatures are valid.
pub fn check_double_voting_proof<Number, Id, MsgHash>(
	report: &DoubleVotingProof<Number, Id, <Id as RuntimeAppPublic>::Signature>,
) -> bool
where
	Id: BeefyAuthorityId<MsgHash> + PartialEq,
	Number: Clone + Encode + PartialEq,
	MsgHash: HashT,
{
	let first = &report.first;
	let second = &report.second;

	// if votes
	//   come from different authorities,
	//   are for different rounds,
	//   have different validator set ids,
	//   or both votes have the same commitment,
	//     --> the equivocation is invalid.
	if first.id != second.id ||
		first.commitment.block_number != second.commitment.block_number ||
		first.commitment.validator_set_id != second.commitment.validator_set_id ||
		first.commitment.payload == second.commitment.payload
	{
		return false
	}

	// check signatures on both votes are valid
	let valid_first = check_commitment_signature(&first.commitment, &first.id, &first.signature);
	let valid_second =
		check_commitment_signature(&second.commitment, &second.id, &second.signature);

	return valid_first && valid_second
}

/// Checks whether the provided header's payload differs from the commitment's payload.
fn check_header_proof<Header>(
	commitment: &Commitment<Header::Number>,
	canonical_header: &Header,
	canonical_header_hash: &Header::Hash,
) -> bool
where
	Header: HeaderT,
{
	let canonical_mmr_root_digest = mmr::find_mmr_root_digest::<Header>(canonical_header);
	let canonical_payload = canonical_mmr_root_digest
		.map(|mmr_root| Payload::from_single_entry(known_payloads::MMR_ROOT_ID, mmr_root.encode()));
	// Check header's hash and that the `payload` of the `commitment` differs from the
	// `canonical_payload`. Note that if the signatories signed a payload when there should be
	// none (for instance for a block prior to BEEFY activation), then canonical_payload = None,
	// and they will likewise be slashed.
	// Note that we can only check this if a valid header has been provided - we cannot
	// slash for this with an ancestry proof - by necessity)
	return canonical_header.hash() == *canonical_header_hash &&
		Some(&commitment.payload) != canonical_payload.as_ref()
}

/// Checks whether an ancestry proof has the correct size and its calculated root differs from the
/// commitment's payload's.
fn check_ancestry_proof<Header, NodeHash>(
	commitment: &Commitment<Header::Number>,
	ancestry_proof: &AncestryProof<NodeHash::Output>,
	first_mmr_block_num: Header::Number,
	best_root: NodeHash::Output,
	mmr_size: u64,
) -> bool
where
	Header: HeaderT,
	NodeHash: HashT,
{
	let expected_leaf_count = sp_mmr_primitives::utils::block_num_to_leaf_index::<Header>(
		commitment.block_number,
		first_mmr_block_num,
	)
	.and_then(|leaf_index| {
		leaf_index.checked_add(1).ok_or_else(|| {
			sp_mmr_primitives::Error::InvalidNumericOp.log_debug("leaf_index + 1 overflowed")
		})
	});

	if let Ok(expected_leaf_count) = expected_leaf_count {
		let expected_mmr_size =
			sp_mmr_primitives::utils::NodesUtils::new(expected_leaf_count).size();
		// verify that the prev_root is at the correct block number
		// this can be inferred from the leaf_count / mmr_size of the prev_root:
		// we've converted the commitment.block_number to an mmr size and now
		// compare with the value in the ancestry proof
		if expected_mmr_size != ancestry_proof.prev_size {
			return false
		}
		if sp_mmr_primitives::utils::verify_ancestry_proof::<
			NodeHash::Output,
			utils::AncestryHasher<NodeHash>,
		>(best_root, mmr_size, ancestry_proof.clone()) !=
			Ok(true)
		{
			return false
		}
	} else {
		// if the block number either under- or overflowed, the
		// commitment.block_number was not valid and the commitment should not have
		// been signed, hence we can skip the ancestry proof and slash the
		// signatories
		return true
	}

	// once the ancestry proof is verified, calculate the prev_root to compare it
	// with the commitment's prev_root
	let ancestry_prev_root = mmr_lib::ancestry_proof::bagging_peaks_hashes::<
		NodeHash::Output,
		AncestryHasher<NodeHash>,
	>(ancestry_proof.prev_peaks.clone());
	// if the commitment payload does not commit to an MMR root, then this
	// commitment may have another purpose and should not be slashed
	let commitment_prev_root =
		commitment.payload.get_decoded::<NodeHash::Output>(&known_payloads::MMR_ROOT_ID);
	return commitment_prev_root != ancestry_prev_root.ok()
}

/// New BEEFY validator set notification hook.
pub trait OnNewValidatorSet<AuthorityId> {
	/// Function called by the pallet when BEEFY validator set changes.
	fn on_new_validator_set(
		validator_set: &ValidatorSet<AuthorityId>,
		next_validator_set: &ValidatorSet<AuthorityId>,
	);
}

/// No-op implementation of [OnNewValidatorSet].
impl<AuthorityId> OnNewValidatorSet<AuthorityId> for () {
	fn on_new_validator_set(_: &ValidatorSet<AuthorityId>, _: &ValidatorSet<AuthorityId>) {}
}

/// Hook for checking fork equivocation proof for validity.
pub trait CheckForkEquivocationProof<Err, Header: HeaderT> {
	/// Associated hash type for hashing ancestry proof.
	type Hash: HashT;
	/// Validate equivocation proof (check commitment is to unexpected payload and
	/// signatures are valid).
	/// NOTE: Fork equivocation proof currently only prevents attacks
	/// assuming 2/3rds of validators honestly participate in BEEFY
	/// finalization and at least one honest relayer can update the
	/// beefy light client at least once every 4096 blocks. See
	/// <https://github.com/paritytech/polkadot-sdk/issues/1441> for
	/// replacement solution.
	fn check_fork_equivocation_proof<Id, MsgHash>(
		proof: &ForkEquivocationProof<Id, Header, <Self::Hash as HashT>::Output>,
	) -> Result<(), Err>
	where
		Id: BeefyAuthorityId<MsgHash> + PartialEq,
		MsgHash: HashT;
}

/// An opaque type used to represent the key ownership proof at the runtime API
/// boundary. The inner value is an encoded representation of the actual key
/// ownership proof which will be parameterized when defining the runtime. At
/// the runtime API boundary this type is unknown and as such we keep this
/// opaque representation, implementors of the runtime API will have to make
/// sure that all usages of `OpaqueKeyOwnershipProof` refer to the same type.
#[derive(Decode, Encode, PartialEq, TypeInfo, Clone)]
pub struct OpaqueKeyOwnershipProof(Vec<u8>);
impl OpaqueKeyOwnershipProof {
	/// Create a new `OpaqueKeyOwnershipProof` using the given encoded
	/// representation.
	pub fn new(inner: Vec<u8>) -> OpaqueKeyOwnershipProof {
		OpaqueKeyOwnershipProof(inner)
	}

	/// Try to decode this `OpaqueKeyOwnershipProof` into the given concrete key
	/// ownership proof type.
	pub fn decode<T: Decode>(self) -> Option<T> {
		codec::Decode::decode(&mut &self.0[..]).ok()
	}
}

sp_api::decl_runtime_apis! {
	/// API necessary for BEEFY voters. Due to the significant conceptual
	/// overlap, in large part, this is lifted from the GRANDPA API.
	#[api_version(4)]
	pub trait BeefyApi<AuthorityId, Hash> where
		AuthorityId : Codec + RuntimeAppPublic,
		Hash: Codec,
	{
		/// Return the block number where BEEFY consensus is enabled/started
		fn beefy_genesis() -> Option<NumberFor<Block>>;

		/// Return the current active BEEFY validator set
		fn validator_set() -> Option<ValidatorSet<AuthorityId>>;

		/// Submits an unsigned extrinsic to report an equivocation. The caller
		/// must provide the equivocation proof and a key ownership proof
		/// (should be obtained using `generate_key_ownership_proof`). The
		/// extrinsic will be unsigned and should only be accepted for local
		/// authorship (not to be broadcast to the network). This method returns
		/// `None` when creation of the extrinsic fails, e.g. if equivocation
		/// reporting is disabled for the given runtime (i.e. this method is
		/// hardcoded to return `None`). Only useful in an offchain context.
		fn submit_report_vote_equivocation_unsigned_extrinsic(
			equivocation_proof:
				DoubleVotingProof<NumberFor<Block>, AuthorityId, <AuthorityId as RuntimeAppPublic>::Signature>,
			key_owner_proof: OpaqueKeyOwnershipProof,
		) -> Option<()>;

		/// Submits an unsigned extrinsic to report commitments to an invalid
		/// fork. The caller must provide the invalid commitments proof and key
		/// ownership proofs (should be obtained using
		/// `generate_key_ownership_proof`) for the offenders. The extrinsic
		/// will be unsigned and should only be accepted for local authorship
		/// (not to be broadcast to the network). This method returns `None`
		/// when creation of the extrinsic fails, e.g. if the key owner proofs
		/// are not validly encoded or if equivocation reporting is disabled for
		/// the given runtime (i.e. this method is hardcoded to return `None`).
		/// Only useful in an offchain context.
		fn submit_report_fork_equivocation_unsigned_extrinsic(
			fork_equivocation_proof:
				ForkEquivocationProof<AuthorityId, Block::Header, Hash>,
			key_owner_proofs: Vec<OpaqueKeyOwnershipProof>,
		) -> Option<()>;

		/// Generates a proof of key ownership for the given authority in the
		/// given set. An example usage of this module is coupled with the
		/// session historical module to prove that a given authority key is
		/// tied to a given staking identity during a specific session. Proofs
		/// of key ownership are necessary for submitting equivocation reports.
		/// NOTE: even though the API takes a `set_id` as parameter the current
		/// implementations ignores this parameter and instead relies on this
		/// method being called at the correct block height, i.e. any point at
		/// which the given set id is live on-chain. Future implementations will
		/// instead use indexed data through an offchain worker, not requiring
		/// older states to be available.
		fn generate_key_ownership_proof(
			set_id: ValidatorSetId,
			authority_id: AuthorityId,
		) -> Option<OpaqueKeyOwnershipProof>;
	}

}

#[cfg(test)]
mod tests {
	use super::*;
	use sp_application_crypto::ecdsa::{self, Public};
	use sp_core::crypto::{Pair, Wraps};
	use sp_crypto_hashing::{blake2_256, keccak_256};
	use sp_runtime::traits::{BlakeTwo256, Keccak256};

	#[test]
	fn validator_set() {
		// Empty set not allowed.
		assert_eq!(ValidatorSet::<Public>::new(vec![], 0), None);

		let alice = ecdsa::Pair::from_string("//Alice", None).unwrap();
		let set_id = 0;
		let validators = ValidatorSet::<Public>::new(vec![alice.public()], set_id).unwrap();

		assert_eq!(validators.id(), set_id);
		assert_eq!(validators.validators(), &vec![alice.public()]);
	}

	#[test]
	fn ecdsa_beefy_verify_works() {
		let msg = &b"test-message"[..];
		let (pair, _) = ecdsa_crypto::Pair::generate();

		let keccak_256_signature: ecdsa_crypto::Signature =
			pair.as_inner_ref().sign_prehashed(&keccak_256(msg)).into();

		let blake2_256_signature: ecdsa_crypto::Signature =
			pair.as_inner_ref().sign_prehashed(&blake2_256(msg)).into();

		// Verification works if same hashing function is used when signing and verifying.
		assert!(BeefyAuthorityId::<Keccak256>::verify(&pair.public(), &keccak_256_signature, msg));
		assert!(BeefyAuthorityId::<BlakeTwo256>::verify(
			&pair.public(),
			&blake2_256_signature,
			msg
		));
		// Verification fails if distinct hashing functions are used when signing and verifying.
		assert!(!BeefyAuthorityId::<Keccak256>::verify(&pair.public(), &blake2_256_signature, msg));
		assert!(!BeefyAuthorityId::<BlakeTwo256>::verify(
			&pair.public(),
			&keccak_256_signature,
			msg
		));

		// Other public key doesn't work
		let (other_pair, _) = ecdsa_crypto::Pair::generate();
		assert!(!BeefyAuthorityId::<Keccak256>::verify(
			&other_pair.public(),
			&keccak_256_signature,
			msg,
		));
		assert!(!BeefyAuthorityId::<BlakeTwo256>::verify(
			&other_pair.public(),
			&blake2_256_signature,
			msg,
		));
	}

	#[test]
	#[cfg(feature = "bls-experimental")]
	fn bls_beefy_verify_works() {
		let msg = &b"test-message"[..];
		let (pair, _) = bls_crypto::Pair::generate();

		let signature: bls_crypto::Signature = pair.as_inner_ref().sign(&msg).into();

		// Verification works if same hashing function is used when signing and verifying.
		assert!(BeefyAuthorityId::<Keccak256>::verify(&pair.public(), &signature, msg));

		// Other public key doesn't work
		let (other_pair, _) = bls_crypto::Pair::generate();
		assert!(!BeefyAuthorityId::<Keccak256>::verify(&other_pair.public(), &signature, msg,));
	}

	#[test]
	#[cfg(feature = "bls-experimental")]
	fn ecdsa_bls_beefy_verify_works() {
		let msg = &b"test-message"[..];
		let (pair, _) = ecdsa_bls_crypto::Pair::generate();

		let signature: ecdsa_bls_crypto::Signature =
			pair.as_inner_ref().sign_with_hasher::<Keccak256>(&msg).into();

		// Verification works if same hashing function is used when signing and verifying.
		assert!(BeefyAuthorityId::<Keccak256>::verify(&pair.public(), &signature, msg));

		// Verification doesn't work if we verify function provided by pair_crypto implementation
		assert!(!ecdsa_bls_crypto::Pair::verify(&signature, msg, &pair.public()));

		// Other public key doesn't work
		let (other_pair, _) = ecdsa_bls_crypto::Pair::generate();
		assert!(!BeefyAuthorityId::<Keccak256>::verify(&other_pair.public(), &signature, msg,));
	}
}
