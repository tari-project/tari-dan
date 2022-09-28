//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#[template]
mod account_extension_template {
    pub struct FungibleAccountExtension {
        base: FungibleAccount,
        // owner_badge: AccessBadge,
    }
    pub fn extend_erc20(base_resource: Something) -> ComponentAddress  {
        let base_erc20 = get_resource(asfasdafsd);
        base_erc20.register_extension();
    }
}


Character (nft);


// DiabloCharacter nft
// DiabloCharacter::<Mikesworld>::{skinId, 3d}

pub struct SportsCard {
    id: u32,
    name: String
}
#[template]
mod sports_cards {

   struct SportsCardContract {
       cards: Vault<SportsCard>,
    }

    impl SportsCardContract {
        fn construct() -> Self {
            todo!()
        }
    }
}

pub struct SportsCardBettingData {
    instance: Ref<SportsCard>,
    // only can be edited by oracle
    rugby_world_cup_wins: u32,
    rugby_world_cup_losses: u32,
    note: String
}


pub struct Team {
    name: String,
    players: Vec<ProofOfAvailablity<SportsCard>>,
}

#[template]
mod sports_cards_betting {
    pub struct SportsCardBetting{
        // ref

        winnings: Vault<FungibleCoin>
    }

    impl SportsCardBetting {
        fn construct() -> Self {
            todo!()
        }

        fn set_data(&mut self, data: SportsCardBettingData) {
            // prove that you're the owner....

            todo!()
        }
    }
}
