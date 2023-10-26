//   Copyright 2022. The Tari Project
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

use tari_template_lib::prelude::*;

#[template]
mod random_template {
    use super::*;

    pub struct RandomTest {
        random: u32,
        random_bytes: Vec<u8>,
        random_long_bytes: Vec<u8>,
    }

    impl RandomTest {
        pub fn create() -> Component<Self> {
            let random: u32 = rand::random_u32();
            let random_bytes = rand::random_bytes(32);
            let random_long_bytes = rand::random_bytes(300);
            Component::new(Self {
                random,
                random_bytes,
                random_long_bytes,
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn get_random(&self) -> u32 {
            self.random
        }

        pub fn get_random_bytes(&self) -> Vec<u8> {
            self.random_bytes.clone()
        }

        pub fn get_random_long_bytes(&self) -> Vec<u8> {
            self.random_long_bytes.clone()
        }
    }
}
