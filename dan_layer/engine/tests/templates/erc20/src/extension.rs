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
