//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::TcpListener, time::Duration};

pub fn get_os_assigned_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

pub fn get_os_assigned_ports() -> (u16, u16) {
    (get_os_assigned_port(), get_os_assigned_port())
}

pub async fn wait_listener_on_local_port(port: u16) {
    let mut i = 0;
    while let Err(e) = tokio::net::TcpSocket::new_v4()
        .unwrap()
        .connect(([127u8, 0, 0, 1], port).into())
        .await
    {
        println!("Waiting for base node to start listening on port {}. {}", port, e);
        if i >= 10 {
            println!("Node failed to start listening on port {} within 10s", port);
            panic!("Node failed to start listening on port {} within 10s", port);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        i += 1;
    }
}
