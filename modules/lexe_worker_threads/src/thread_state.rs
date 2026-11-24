// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::cell::Cell;

thread_local! {
    static WORKER_THREAD_ID: Cell<u64> = const { Cell::new(0) };
}

/// Main thread uses id 0; each worker OS thread sets a non-zero id before running JS.
pub fn set_worker_thread_id(id: u64) {
    WORKER_THREAD_ID.with(|c| c.set(id));
}

pub fn worker_thread_id() -> u64 {
    WORKER_THREAD_ID.with(|c| c.get())
}

pub fn worker_is_main_thread() -> bool {
    worker_thread_id() == 0
}
