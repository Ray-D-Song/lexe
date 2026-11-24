// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::ffi::CString;

use crate::types::napi_status;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastError {
    pub status: napi_status,
    pub message: Option<CString>,
}

impl Default for LastError {
    fn default() -> Self {
        Self {
            status: napi_status::napi_ok,
            message: None,
        }
    }
}
