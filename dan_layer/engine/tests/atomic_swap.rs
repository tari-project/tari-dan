//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_test_tooling::TemplateTest;

#[test]
fn successfull_swap() {
    let _test = TemplateTest::new(vec!["../template_builtin/templates/atomic_swap/"]);
}
