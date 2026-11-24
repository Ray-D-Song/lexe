// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

mod client;
mod constants;
mod server;
mod stream;

pub use server::{create_secure_server, create_server, Http2Module, Http2Server};

#[cfg(test)]
mod server_tests;
