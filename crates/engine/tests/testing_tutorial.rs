//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Worked examples for the "Testing your template" guide
//! (`docs/developer-docs/src/content/docs/guides/testing-templates.mdx`).
//!
//! The guide embeds the code below verbatim, so these tests are what keeps it honest:
//! if an API in the guide changes, one of these tests stops compiling or stops passing.
//! [`guide_snippets_match_their_source`] checks that the page and its sources have not drifted
//! apart.

use std::{fs, path::Path};

use tari_ootle_transaction::args;
use tari_template_lib::{
    prelude::TARI_TOKEN,
    types::{Amount, NonFungibleId},
};
use tari_template_test_tooling::{TemplateTest, support::assert_error::assert_reject_reason};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn create_accounts_and_call_a_component() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/guessing_game"]);
    let template = test.get_template_address("GuessingGame");

    // Creates a fresh key pair, publishes an account component for it and funds it from the
    // built-in XTR faucet.
    let (player, _player_proof, player_key) = test.create_funded_account();

    let vaults = test.read_only_state_store().get_vaults_for_account(player).unwrap();
    assert_eq!(
        vaults[&TARI_TOKEN].balance(),
        Amount::from(TemplateTest::FUNDED_ACCOUNT_INITIAL_BALANCE)
    );

    // `start_game` is owner-restricted, so the transaction is sealed by the key that owns the
    // component - the harness's own key, which created it in the same transaction.
    test.execute_expect_success(
        test.transaction()
            .allocate_component_address("game")
            .call_function(template, "new", args![Workspace("game")])
            .call_method("game", "start_game", args![NonFungibleId::from_string("💎")])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let (game, _) = test
        .read_only_state_store()
        .get_components_by_template_address(template)
        .unwrap()
        .remove(0);

    // The player seals this one with their own key. `guess` is declared `rule![allow_all]`, so no
    // ownership proof is required: with an empty `proofs` argument the harness derives the proofs
    // from the transaction signers.
    test.execute_expect_success(
        test.transaction()
            .call_method(game, "guess", args![5u8, player])
            .build_and_seal(&player_key),
        vec![],
    );
}

#[test]
fn inspect_component_state_directly() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/guessing_game"]);
    let template = test.get_template_address("GuessingGame");

    test.execute_expect_success(
        test.transaction()
            .allocate_component_address("game")
            .call_function(template, "new", args![Workspace("game")])
            .call_method("game", "start_game", args![NonFungibleId::from_string("💎")])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let (game, _) = test
        .read_only_state_store()
        .get_components_by_template_address(template)
        .unwrap()
        .remove(0);

    // Component state is encoded field by field, so a path segment is the *index* of the field in
    // the template struct. `GuessingGame` declares `prize_vault`, `guesses`, `round_number`, so
    // `$.2` is the round counter that `start_game` incremented.
    let round_number: u32 = test.extract_component_value(game, "$.2");
    assert_eq!(round_number, 1);

    // The store also answers typed questions about a component. `start_game` minted the prize NFT
    // into the component's vault, so that vault now holds exactly one token.
    let vaults = test.read_only_state_store().get_vaults_for_component(game).unwrap();
    let prize_vault = vaults.values().next().expect("the game holds a prize vault");
    assert_eq!(prize_vault.balance(), Amount::from(1u64));
}

#[test]
fn assert_on_a_rejected_transaction() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/guessing_game"]);
    let template = test.get_template_address("GuessingGame");

    test.execute_expect_success(
        test.transaction()
            .allocate_component_address("game")
            .call_function(template, "new", args![Workspace("game")])
            .call_method("game", "start_game", args![NonFungibleId::from_string("💎")])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let (game, _) = test
        .read_only_state_store()
        .get_components_by_template_address(template)
        .unwrap()
        .remove(0);

    let (player, _, player_key) = test.create_funded_account();
    let (cheat, _, cheat_key) = test.create_funded_account();

    // A guess that the template accepts is recorded in `guesses`. Establishing that first is what
    // makes the rejection below meaningful: this is the observable a committed `guess` changes.
    let before_any_guess = test.read_only_state_store().get_component(game).unwrap();
    test.execute_expect_success(
        test.transaction()
            .call_method(game, "guess", args![5u8, player])
            .build_and_seal(&player_key),
        vec![],
    );
    let after_valid_guess = test.read_only_state_store().get_component(game).unwrap();
    assert_ne!(
        after_valid_guess.state(),
        before_any_guess.state(),
        "an accepted guess must be visible in the component state"
    );

    // The template asserts `guess <= 10`, and the panic message reaches the test verbatim.
    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(game, "guess", args![100u8, cheat])
            .build_and_seal(&cheat_key),
        vec![],
    );
    assert_reject_reason(reason, "Template error: Guess must be from 0 to 10");

    // A rejected transaction commits nothing: the same state the accepted guess produced.
    let after_rejection = test.read_only_state_store().get_component(game).unwrap();
    assert_eq!(
        after_rejection.state(),
        after_valid_guess.state(),
        "a rejected transaction must not commit state"
    );
}

/// Every Rust block in the guide carries the path of the file it was copied from. Compare the two,
/// ignoring indentation, so that an example cannot silently drift away from the code that this
/// test suite actually compiles and runs.
#[test]
fn guide_snippets_match_their_source() {
    let repo_root = Path::new(CRATE_PATH).join("../..");
    let guide_path = repo_root.join("docs/developer-docs/src/content/docs/guides/testing-templates.mdx");
    let guide = fs::read_to_string(&guide_path).expect("guide is missing");

    let mut checked = 0;
    for (title, snippet) in rust_blocks_with_a_source_path(&guide) {
        let source = fs::read_to_string(repo_root.join(&title)).unwrap_or_else(|e| panic!("{title}: {e}"));
        assert!(
            contains_lines(&source, &snippet),
            "{}: a snippet in the guide is not present in {title} verbatim:\n{snippet}",
            guide_path.display()
        );
        checked += 1;
    }
    assert!(checked > 0, "no source-backed Rust blocks found in the guide");
}

/// Yields `(title, contents)` for each ```rust block in the guide that names a source file.
fn rust_blocks_with_a_source_path(guide: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut lines = guide.lines();
    while let Some(line) = lines.next() {
        let Some(meta) = line.strip_prefix("```rust ") else {
            continue;
        };
        let Some(title) = meta.split_once("title=\"").and_then(|(_, t)| t.split_once('"')) else {
            continue;
        };
        let contents = lines
            .by_ref()
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n");
        blocks.push((title.0.to_string(), contents));
    }
    blocks
}

/// True if the non-blank lines of `snippet`, trimmed, appear consecutively in `source`.
fn contains_lines(source: &str, snippet: &str) -> bool {
    let trimmed = |s: &str| {
        s.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let source = trimmed(source);
    let snippet = trimmed(snippet);
    !snippet.is_empty() && source.windows(snippet.len()).any(|w| w == snippet)
}
