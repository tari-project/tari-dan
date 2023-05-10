//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, TokenData, Validation};
use serde::{Deserialize, Serialize};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

pub struct Data {
    pub offers: HashMap<u64, String>,
    pub answers: HashMap<u64, String>,
    pub offer_ice_candidates: HashMap<u64, Vec<RTCIceCandidateInit>>,
    pub answer_ice_candidates: HashMap<u64, Vec<RTCIceCandidateInit>>,
    pub expiration: Duration,
    pub secret_key: String,
    // The lowest still probably alive id. They will be cleaned up after expiration, which goes in order.
    pub low_id: u64,
    pub id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    id: u64,
    exp: usize,
}

impl Data {
    pub fn new() -> Self {
        Data {
            offers: HashMap::new(),
            answers: HashMap::new(),
            offer_ice_candidates: HashMap::new(),
            answer_ice_candidates: HashMap::new(),
            expiration: Duration::minutes(5),
            secret_key: "secret_key".into(),
            low_id: 0,
            id: 0,
        }
    }

    pub fn generate_jwt(&mut self) -> Result<String> {
        let my_claims = Claims {
            id: self.id,
            exp: (Utc::now() + self.expiration).timestamp() as usize,
        };
        self.id += 1;
        encode(
            &Header::default(),
            &my_claims,
            &EncodingKey::from_secret(self.secret_key.as_ref()),
        )
        .map_err(anyhow::Error::new)
    }

    pub fn check_jwt(&self, token: String) -> Result<u64> {
        let token: TokenData<Claims> = decode(
            &token,
            &DecodingKey::from_secret(self.secret_key.as_ref()),
            &Validation::default(),
        )?;
        Ok(token.claims.id)
    }

    pub fn add_offer(&mut self, id: u64, offer: String) {
        self.offers.insert(id, offer);
    }

    pub fn get_offer(&self, id: u64) -> Result<&String> {
        self.offers.get(&id).ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }

    pub fn add_answer(&mut self, id: u64, offer: String) {
        self.answers.insert(id, offer);
    }

    pub fn get_answer(&self, id: u64) -> Result<&String> {
        self.answers.get(&id).ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }

    pub fn add_offer_ice_candidate(&mut self, id: u64, ice_candidate: RTCIceCandidateInit) {
        self.offer_ice_candidates
            .entry(id)
            .or_insert_with(Vec::new)
            .push(ice_candidate);
    }

    pub fn get_offer_ice_candidates(&self, id: u64) -> Result<&Vec<RTCIceCandidateInit>> {
        self.offer_ice_candidates
            .get(&id)
            .ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }

    pub fn add_answer_ice_candidate(&mut self, id: u64, ice_candidate: RTCIceCandidateInit) {
        self.answer_ice_candidates
            .entry(id)
            .or_insert_with(Vec::new)
            .push(ice_candidate);
    }

    pub fn get_answer_ice_candidates(&self, id: u64) -> Result<&Vec<RTCIceCandidateInit>> {
        self.answer_ice_candidates
            .get(&id)
            .ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }
}
