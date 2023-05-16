//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, net::TcpListener, time::Duration};

pub fn get_os_assigned_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

pub fn get_os_assigned_ports() -> (u16, u16) {
    (get_os_assigned_port(), get_os_assigned_port())
}

pub async fn wait_listener_on_local_port(port: u16) {
    let mut i = 0;
    while let Err(_e) = tokio::net::TcpSocket::new_v4()
        .unwrap()
        .connect(([127u8, 0, 0, 1], port).into())
        .await
    {
        // println!("Waiting for base node to start listening on port {}. {}", port, e);
        if i >= 10 {
            // println!("Node failed to start listening on port {} within 10s", port);
            panic!("Node failed to start listening on port {} within 10s", port);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        i += 1;
    }
}

pub async fn check_join_handle<E: Display>(
    name: &str,
    handle: tokio::task::JoinHandle<Result<(), E>>,
) -> tokio::task::JoinHandle<Result<(), E>> {
    if !handle.is_finished() {
        return handle;
    }

    match handle.await {
        Ok(Ok(_)) => {
            panic!("Node {} exited unexpectedly", name);
        },
        Ok(Err(e)) => {
            panic!("Node {} exited unexpectedly with error: {}", name, e);
        },
        Err(e) => {
            panic!("Node {} panicked: {:?}", name, e.try_into_panic());
        },
    }
}
