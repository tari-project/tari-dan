//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, TokenData, Validation};
use serde::{Deserialize, Serialize};
use serde_json as json;
use tari_dan_common_types::crypto::create_secret;
use tari_dan_wallet_sdk::apis::jwt::JrpcPermissions;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

pub struct Data {
    pub offers: HashMap<u64, json::Value>,
    pub answers: HashMap<u64, json::Value>,
    pub offer_ice_candidates: HashMap<u64, Vec<RTCIceCandidateInit>>,
    pub answer_ice_candidates: HashMap<u64, Vec<RTCIceCandidateInit>>,
    pub expiration: chrono::Duration,
    pub secret_key: String,
    // The lowest still probably alive id. They will be cleaned up after expiration, which goes in order.
    pub low_id: u64,
    pub id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    id: u64,
    name: String,
    permissions: JrpcPermissions,
    exp: usize,
}

impl Data {
    pub fn new() -> Self {
        Data {
            offers: HashMap::new(),
            answers: HashMap::new(),
            offer_ice_candidates: HashMap::new(),
            answer_ice_candidates: HashMap::new(),
            expiration: chrono::Duration::minutes(5),
            secret_key: create_secret(),
            low_id: 0,
            id: 0,
        }
    }

    pub fn with_expiration(mut self, expiration: Duration) -> Self {
        self.expiration = chrono::Duration::from_std(expiration).unwrap();
        self
    }

    pub fn generate_jwt(&mut self, permissions: JrpcPermissions) -> anyhow::Result<String> {
        let my_claims = Claims {
            id: self.id,
            name: self.id.to_string(),
            permissions,
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

    pub fn check_jwt(&self, token: String) -> anyhow::Result<u64> {
        let token: TokenData<Claims> = decode(
            &token,
            &DecodingKey::from_secret(self.secret_key.as_ref()),
            &Validation::default(),
        )?;
        Ok(token.claims.id)
    }

    pub fn add_offer(&mut self, id: u64, offer: json::Value) {
        self.offers.insert(id, offer);
    }

    pub fn get_offer(&self, id: u64) -> anyhow::Result<&json::Value> {
        self.offers.get(&id).ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }

    pub fn add_answer(&mut self, id: u64, offer: json::Value) {
        self.answers.insert(id, offer);
    }

    pub fn get_answer(&self, id: u64) -> anyhow::Result<&json::Value> {
        self.answers.get(&id).ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }

    pub fn add_offer_ice_candidate(&mut self, id: u64, ice_candidate: RTCIceCandidateInit) {
        self.offer_ice_candidates.entry(id).or_default().push(ice_candidate);
    }

    pub fn get_offer_ice_candidates(&self, id: u64) -> anyhow::Result<&Vec<RTCIceCandidateInit>> {
        self.offer_ice_candidates
            .get(&id)
            .ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }

    pub fn add_answer_ice_candidate(&mut self, id: u64, ice_candidate: RTCIceCandidateInit) {
        self.answer_ice_candidates.entry(id).or_default().push(ice_candidate);
    }

    pub fn get_answer_ice_candidates(&self, id: u64) -> anyhow::Result<&Vec<RTCIceCandidateInit>> {
        self.answer_ice_candidates
            .get(&id)
            .ok_or_else(|| anyhow::Error::msg("Invalid id"))
    }
}
