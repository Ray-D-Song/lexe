// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
pub use self::base::*;

#[allow(clippy::module_inception)]
mod base {
    pub use lexe_core::bytecode;
    pub use lexe_core::compiler;
    pub use lexe_core::environment;
    pub use lexe_core::libs;
    pub use lexe_core::modules;
    #[allow(unused_imports)]
    pub use lexe_core::vm;
}
pub use lexe_core::VERSION;

// rquickjs components
#[allow(unused_imports)]
pub use lexe_core::{
    atom::PredefinedAtom, context::EvalOptions, function::Rest, AsyncContext, CatchResultExt, Ctx,
    Error, Object, Promise,
};
