// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `Sign`, `Verify`, `createSign`, and `createVerify`.

use lexe_utils::result::ResultExt;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, prelude::This, Class, Ctx, Exception,
    JsLifetime, Result, Value,
};

use crate::{
    classic::{
        parse_sign_digest_algorithm, parse_sign_verify_key, sign_material_bytes,
        verify_material_bytes,
    },
    hash::HashAlgorithm,
    key_object::{encode_bytes_to_output, optional_encoding, value_to_bytes_with_encoding},
};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Sign {
    #[qjs(skip_trace)]
    hash: HashAlgorithm,
    data: Vec<u8>,
    finalized: bool,
}

impl Sign {
    pub fn new(ctx: Ctx<'_>, algorithm: Value<'_>) -> Result<Self> {
        let hash = parse_sign_digest_algorithm(&ctx, &algorithm)?
            .ok_or_else(|| Exception::throw_type(&ctx, "createSign requires a digest algorithm"))?;
        Ok(Self {
            hash,
            data: Vec::new(),
            finalized: false,
        })
    }
}

#[rquickjs::methods]
impl Sign {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'_>, algorithm: Value<'_>) -> Result<Self> {
        Self::new(ctx, algorithm)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Sign"
    }

    pub fn update<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        data: Value<'js>,
        input_encoding: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let enc = optional_encoding(&ctx, input_encoding.0)?;
        let bytes = value_to_bytes_with_encoding(&ctx, &data, enc.as_deref(), "data")?;
        {
            let mut inner = this.0.borrow_mut();
            if inner.finalized {
                return Err(Exception::throw_message(&ctx, "Sign already finalized"));
            }
            inner.data.extend_from_slice(&bytes);
        }
        Ok(this.0)
    }

    pub fn sign<'js>(
        &mut self,
        ctx: Ctx<'js>,
        private_key: Value<'js>,
        output_format: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        if self.finalized {
            return Err(Exception::throw_message(&ctx, "Sign already finalized"));
        }
        let parsed = parse_sign_verify_key(&ctx, private_key, true)?;
        let out_enc = optional_encoding(&ctx, output_format.0)?;
        let sig = sign_material_bytes(
            &parsed.key.material,
            Some(self.hash),
            &self.data,
            parsed.padding,
            parsed.salt_length,
            parsed.dsa_encoding,
        )
        .or_throw(&ctx)?;
        self.finalized = true;
        self.data.clear();
        encode_bytes_to_output(&ctx, sig, out_enc.as_deref())
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Verify {
    #[qjs(skip_trace)]
    hash: HashAlgorithm,
    data: Vec<u8>,
    finalized: bool,
}

impl Verify {
    pub fn new(ctx: Ctx<'_>, algorithm: Value<'_>) -> Result<Self> {
        let hash = parse_sign_digest_algorithm(&ctx, &algorithm)?.ok_or_else(|| {
            Exception::throw_type(&ctx, "createVerify requires a digest algorithm")
        })?;
        Ok(Self {
            hash,
            data: Vec::new(),
            finalized: false,
        })
    }
}

#[rquickjs::methods]
impl Verify {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'_>, algorithm: Value<'_>) -> Result<Self> {
        Self::new(ctx, algorithm)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Verify"
    }

    pub fn update<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        data: Value<'js>,
        input_encoding: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let enc = optional_encoding(&ctx, input_encoding.0)?;
        let bytes = value_to_bytes_with_encoding(&ctx, &data, enc.as_deref(), "data")?;
        {
            let mut inner = this.0.borrow_mut();
            if inner.finalized {
                return Err(Exception::throw_message(&ctx, "Verify already finalized"));
            }
            inner.data.extend_from_slice(&bytes);
        }
        Ok(this.0)
    }

    pub fn verify<'js>(
        &mut self,
        ctx: Ctx<'js>,
        object: Value<'js>,
        signature: Value<'js>,
        signature_format: Opt<Value<'js>>,
    ) -> Result<bool> {
        if self.finalized {
            return Err(Exception::throw_message(&ctx, "Verify already finalized"));
        }
        let parsed = parse_sign_verify_key(&ctx, object, false)?;
        let sig_enc = optional_encoding(&ctx, signature_format.0)?;
        let sig_bytes =
            value_to_bytes_with_encoding(&ctx, &signature, sig_enc.as_deref(), "signature")?;
        let valid = verify_material_bytes(
            &parsed.key.material,
            Some(self.hash),
            &self.data,
            &sig_bytes,
            parsed.padding,
            parsed.salt_length,
            parsed.dsa_encoding,
        )
        .or_throw(&ctx)?;
        self.finalized = true;
        self.data.clear();
        Ok(valid)
    }
}

pub fn create_sign<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    _options: Opt<Value<'js>>,
) -> Result<Class<'js, Sign>> {
    let instance = Sign::new(ctx.clone(), algorithm)?;
    Class::instance(ctx, instance)
}

pub fn create_verify<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    _options: Opt<Value<'js>>,
) -> Result<Class<'js, Verify>> {
    let instance = Verify::new(ctx.clone(), algorithm)?;
    Class::instance(ctx, instance)
}
