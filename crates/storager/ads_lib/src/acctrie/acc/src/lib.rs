#[macro_use]
extern crate lazy_static;

pub mod digest;
pub use digest::*;

pub mod set;
pub use set::*;

pub mod digest_set;
pub use digest_set::*;

pub mod serde_impl;
pub use serde_impl::*;

pub mod utils;
pub use utils::*;

pub mod acc_mod;
pub use acc_mod::*;

// Compatibility shim: expose a `dynamic_accumulator` module with `MembershipProof`
// so other workspace crates (e.g., `common`) can import
// `ads_rust::acctrie::acc::dynamic_accumulator::MembershipProof`.
pub mod dynamic_accumulator {
	use super::acc_mod::G1Affine;
	use super::acc_mod::Fr;

	#[derive(Debug, Clone, Eq, PartialEq)]
	pub struct MembershipProof {
		pub witness: G1Affine,
		pub element: Fr,
	}

	impl MembershipProof {
		pub fn verify(&self, acc: G1Affine) -> bool {
			super::acc_mod::Acc::verify_membership(&acc, &self.witness, &self.element)
		}
	}
}
