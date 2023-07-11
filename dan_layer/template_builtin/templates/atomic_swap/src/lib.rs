//   Copyright 2023. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use digest::Digest;
use tari_crypto::hash::blake2::Blake256;
use tari_template_lib::{prelude::*, Hash};

pub type Preimage = [u8; 32];

#[template]
mod atomic_swap_template {
    use super::*;

    pub struct HashedTimelock {
        locked_funds: Vault,
        sender_token: NonFungibleAddress,
        receiver_token: NonFungibleAddress,
        hashlock: Hash,
        preimage: Option<Preimage>,
        // TODO: we are using epoch number for now, but we will need block/timestamp support eventually
        timelock: u64,
    }

    impl HashedTimelock {
        pub fn create(
            funds: Bucket,
            sender_token: NonFungibleAddress,
            receiver_token: NonFungibleAddress,
            hashlock: Hash,
            timelock: u64,
        ) -> HashedTimelockComponent {
            // funds cannot be empty
            assert!(
                funds.amount() > Amount::zero(),
                "The bucket with the funds cannot be empty"
            );
            let locked_funds = Vault::from_bucket(funds);

            // check that the timelock is valid
            assert!(
                timelock > Consensus::current_epoch(),
                "The timelock must be in the future"
            );

            // only the owner of the receiver account will be able to withdraw funds by revealing the preimage
            let withdraw_rule = AccessRule::Restricted(Require(receiver_token.clone()));

            // and only the owner of the sender account will be able to refund after the timelock
            let refund_rule = AccessRule::Restricted(Require(sender_token.clone()));

            // enforce the security rules on the proper methods
            let rules = AccessRules::new()
                .add_method_rule("withdraw", withdraw_rule)
                .add_method_rule("refund", refund_rule);

            Self {
                locked_funds,
                sender_token: sender_token.clone(),
                receiver_token: receiver_token.clone(),
                hashlock,
                timelock,
                preimage: None,
            }
            .create_with_options(rules, None)
        }

        // called by the receiver of the swap, once they know the hashlock preimage, to retrieve the funds
        pub fn withdraw(&mut self, preimage: Preimage) -> Bucket {
            self.check_hashlock(&preimage);

            // we explicitly store the preimage to make it easier for the other party to retrieve it
            self.preimage = Some(preimage);
            self.locked_funds.withdraw_all()
        }

        // called by the sender of the swap to get back the funds if the swap failed
        pub fn refund(&mut self) -> Bucket {
            self.check_timelock();

            self.locked_funds.withdraw_all()
        }

        pub fn get_sender_public_key(&self) -> RistrettoPublicKeyBytes {
            self.sender_token
                .to_public_key()
                .unwrap_or_else(|| panic!("sender_token is not a valid public key: {}", self.sender_token))
        }

        pub fn get_receiver_public_key(&self) -> RistrettoPublicKeyBytes {
            self.receiver_token
                .to_public_key()
                .unwrap_or_else(|| panic!("receiver_token is not a valid public key: {}", self.receiver_token))
        }

        fn check_hashlock(&self, preimage: &Preimage) {
            // TODO: include domain separation?
            let hashlock: [u8; 32] = Blake256::new().chain(preimage).finalize().into();
            let hashlock: Hash = hashlock.into();

            assert!(self.hashlock == hashlock, "Invalid preimage");
        }

        fn check_timelock(&self) {
            assert!(Consensus::current_epoch() > self.timelock, "Timelock not yet passed");
        }
    }
}
