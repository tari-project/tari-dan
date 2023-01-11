//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::net::TcpListener;

pub fn get_os_assigned_ports() -> (u16, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port1 = listener.local_addr().unwrap().port();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port2 = listener.local_addr().unwrap().port();
    (port1, port2)
}
