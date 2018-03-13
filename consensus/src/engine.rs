// Copyright 2018 Kodebox, Inc.
// This file is part of CodeChain.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::fmt;

use bytes::Bytes;
use codechain_types::{Address, H256};
use keys::Signature;
use rlp::{Encodable, Decodable, DecoderError, RlpStream, UntrustedRlp};

use super::epoch::{EpochVerifier, NoOp};
use super::error::Error;
use super::machine::Machine;

/// Seal type.
#[derive(Debug, PartialEq, Eq)]
pub enum Seal {
    /// Proposal seal; should be broadcasted, but not inserted into blockchain.
    Proposal(Vec<Bytes>),
    /// Regular block seal; should be part of the blockchain.
    Regular(Vec<Bytes>),
    /// Engine does generate seal for this block right now.
    None,
}

/// A consensus mechanism for the chain.
pub trait ConsensusEngine<M: Machine>: Sync + Send {
    /// The name of this engine.
    fn name(&self) -> &str;

    /// Get access to the underlying state machine.
    fn machine(&self) -> &M;

    /// The number of additional header fields required for this engine.
    fn seal_fields(&self, _header: &M::Header) -> usize { 0 }

    /// None means that it requires external input (e.g. PoW) to seal a block.
    /// Some(true) means the engine is currently prime for seal generation (i.e. node is the current validator).
    /// Some(false) means that the node might seal internally but is not qualified now.
    fn seals_internally(&self) -> Option<bool> { None }

    /// Attempt to seal the block internally.
    ///
    /// If `Some` is returned, then you get a valid seal.
    ///
    /// This operation is synchronous and may (quite reasonably) not be available, in which None will
    /// be returned.
    ///
    /// It is fine to require access to state or a full client for this function, since
    /// light clients do not generate seals.
    fn generate_seal(&self, _block: &M::LiveBlock, _parent: &M::Header) -> Seal { Seal::None }

    /// Verify a locally-generated seal of a header.
    ///
    /// If this engine seals internally,
    /// no checks have to be done here, since all internally generated seals
    /// should be valid.
    ///
    /// Externally-generated seals (e.g. PoW) will need to be checked for validity.
    ///
    /// It is fine to require access to state or a full client for this function, since
    /// light clients do not generate seals.
    fn verify_local_seal(&self, header: &M::Header) -> Result<(), M::Error>;

    /// Phase 1 quick block verification. Only does checks that are cheap. Returns either a null `Ok` or a general error detailing the problem with import.
    fn verify_block_basic(&self, _header: &M::Header) -> Result<(), M::Error> { Ok(()) }

    /// Phase 2 verification. Perform costly checks such as transaction signatures. Returns either a null `Ok` or a general error detailing the problem with import.
    fn verify_block_unordered(&self, _header: &M::Header) -> Result<(), M::Error> { Ok(()) }

    /// Phase 3 verification. Check block information against parent. Returns either a null `Ok` or a general error detailing the problem with import.
    fn verify_block_family(&self, _header: &M::Header, _parent: &M::Header) -> Result<(), M::Error> { Ok(()) }

    /// Phase 4 verification. Verify block header against potentially external data.
    /// Should only be called when `register_client` has been called previously.
    fn verify_block_external(&self, _header: &M::Header) -> Result<(), M::Error> { Ok(()) }

    /// Genesis epoch data.
    fn genesis_epoch_data<'a>(&self, _header: &M::Header) -> Result<Vec<u8>, String> { Ok(Vec::new()) }

    /// Whether an epoch change is signalled at the given header but will require finality.
    /// If a change can be enacted immediately then return `No` from this function but
    /// `Yes` from `is_epoch_end`.
    ///
    /// Return `Yes` or `No` when the answer is definitively known.
    fn signals_epoch_end<'a>(&self, _header: &M::Header)
        -> EpochChange {
        EpochChange::No
    }

    /// Whether a block is the end of an epoch.
    ///
    /// This either means that an immediate transition occurs or a block signalling transition
    /// has reached finality. The `Headers` given are not guaranteed to return any blocks
    /// from any epoch other than the current.
    ///
    /// Return optional transition proof.
    fn is_epoch_end(
        &self,
        _chain_head: &M::Header,
        _chain: &Headers<M::Header>,
        _transition_store: &PendingTransitionStore,
    ) -> Option<Vec<u8>> {
        None
    }

    /// Create an epoch verifier from validation proof and a flag indicating
    /// whether finality is required.
    fn epoch_verifier<'a>(&self, _header: &M::Header, _proof: &'a [u8]) -> ConstructedVerifier<'a, M> {
        ConstructedVerifier::Trusted(Box::new(NoOp))
    }

    /// Trigger next step of the consensus engine.
    fn step(&self) {}

    /// Block transformation functions, before the transactions.
    fn on_new_block(
        &self,
        _block: &mut M::LiveBlock,
        _epoch_begin: bool,
    ) -> Result<(), M::Error> {
        Ok(())
    }

    /// Block transformation functions, after the transactions.
    fn on_close_block(&self, _block: &mut M::LiveBlock) -> Result<(), M::Error> {
        Ok(())
    }

    /// Sign using the EngineSigner, to be used for consensus tx signing.
    fn sign(&self, _hash: H256) -> Result<Signature, Error> { unimplemented!() }
}

/// Results of a query of whether an epoch change occurred at the given block.
pub enum EpochChange {
    /// Cannot determine until more data is passed.
    Unsure,
    /// No epoch change.
    No,
    /// The epoch will change, with proof.
    Yes(Proof),
}

/// Proof generated on epoch change.
pub enum Proof {
    /// Known proof (extracted from signal)
    Known(Vec<u8>)
}

/// Generated epoch verifier.
pub enum ConstructedVerifier<'a, M: Machine> {
    /// Fully trusted verifier.
    Trusted(Box<EpochVerifier<M>>),
    /// Verifier unconfirmed. Check whether given finality proof finalizes given hash
/// under previous epoch.
    Unconfirmed(Box<EpochVerifier<M>>, &'a [u8], H256),
    /// Error constructing verifier.
    Err(Error),
}

/// Type alias for a function we can get headers by hash through.
pub type Headers<'a, H> = Fn(H256) -> Option<H> + 'a;

/// Type alias for a function we can query pending transitions by block hash through.
pub type PendingTransitionStore<'a> = Fn(H256) -> Option<PendingTransition> + 'a;

/// An epoch transition pending a finality proof.
/// Not all transitions need one.
pub struct PendingTransition {
    /// "transition/epoch" proof from the engine.
    pub proof: Vec<u8>,
}

impl Encodable for PendingTransition {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.append(&self.proof);
    }
}

impl Decodable for PendingTransition {
    fn decode(rlp: &UntrustedRlp) -> Result<Self, DecoderError> {
        Ok(PendingTransition {
            proof: rlp.as_val()?,
        })
    }
}

/// Voting errors.
#[derive(Debug)]
pub enum EngineError {
    /// Signature or author field does not belong to an authority.
    NotAuthorized(Address),
}


impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::EngineError::*;
        let msg = match *self {
            NotAuthorized(ref address) => format!("Signer {} is not authorized.", address),
        };

        f.write_fmt(format_args!("Engine error ({})", msg))
    }
}
