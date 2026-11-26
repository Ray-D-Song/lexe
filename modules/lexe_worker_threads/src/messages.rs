// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use lexe_utils::{clone_wire::CloneWirePayload, env_store::EnvMap};

#[derive(Debug, Clone)]
pub enum WorkerEnvMode {
    Shared(EnvMap),
    Snapshot(HashMap<String, String>),
}

#[derive(Debug, Clone, Default)]
pub struct WorkerResourceLimits {
    pub max_young_generation_size_mb: Option<f64>,
    pub max_old_generation_size_mb: Option<f64>,
    pub code_range_size_mb: Option<f64>,
    pub stack_size_mb: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum WorkerToMain {
    Online,
    Message(CloneWirePayload),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
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
    StdinData(Vec<u8>),
    StdinEnd,
    Terminate,
}
