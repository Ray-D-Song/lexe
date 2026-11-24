// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_utils::clone_wire::CloneWirePayload;

#[derive(Debug, Clone)]
pub enum WorkerToMain {
    Online,
    Message(CloneWirePayload),
    Error(String),
    Exit { code: i32, signal: Option<String> },
}

#[derive(Debug, Clone)]
pub enum MainToWorker {
    Message(CloneWirePayload),
    EnvPatch {
        key: CloneWirePayload,
        value: CloneWirePayload,
    },
    Terminate,
}
