// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `DiffieHellman`, `DiffieHellmanGroup`, `checkPrime*`, and `generatePrime*`.

use lexe_context::CtxExtension;
use lexe_encoding::bytes_to_hex_string;
use lexe_utils::{bytes::ObjectBytes, error::ErrorExtensions, result::ResultExt};
use num_bigint::BigUint;
use rand::RngExt;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, ArrayBuffer, Class, Ctx, Exception,
    Function, IntoJs, JsLifetime, Null, Result, Value,
};

use crate::{
    key_object::{
        encode_bytes_to_output, optional_encoding, throw_code_error, value_to_bytes_with_encoding,
    },
    rsa::{
        DH_CHECK_P_NOT_PRIME, DH_CHECK_P_NOT_SAFE_PRIME, DH_NOT_SUITABLE_GENERATOR,
        DH_UNABLE_TO_CHECK_GENERATOR,
    },
};

const MODP1_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a63a3620ffffffffffffffff";
const MODP2_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece65381ffffffffffffffff";
const MODP5_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca237327ffffffffffffffff";
const MODP14_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aacaa68ffffffffffffffff";
const MODP15_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a93ad2caffffffffffffffff";
const MODP16_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a92108011a723c12a787e6d788719a10bdba5b2699c327186af4e23c1a946834b6150bda2583e9ca2ad44ce8dbbbc2db04de8ef92e8efc141fbecaa6287c59474e6bc05d99b2964fa090c3a2233ba186515be7ed1f612970cee2d7afb81bdd762170481cd0069127d5b05aa993b4ea988d8fddc186ffb7dc90a6c08f4df435c934063199ffffffffffffffff";
const MODP17_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a92108011a723c12a787e6d788719a10bdba5b2699c327186af4e23c1a946834b6150bda2583e9ca2ad44ce8dbbbc2db04de8ef92e8efc141fbecaa6287c59474e6bc05d99b2964fa090c3a2233ba186515be7ed1f612970cee2d7afb81bdd762170481cd0069127d5b05aa993b4ea988d8fddc186ffb7dc90a6c08f4df435c93402849236c3fab4d27c7026c1d4dcb2602646dec9751e763dba37bdf8ff9406ad9e530ee5db382f413001aeb06a53ed9027d831179727b0865a8918da3edbebcf9b14ed44ce6cbaced4bb1bdb7f1447e6cc254b332051512bd7af426fb8f401378cd2bf5983ca01c64b92ecf032ea15d1721d03f482d7ce6e74fef6d55e702f46980c82b5a84031900b1c9e59e7c97fbec7e8f323a97a7e36cc88be0f1d45b7ff585ac54bd407b22b4154aacc8f6d7ebf48e1d814cc5ed20f8037e0a79715eef29be32806a1d58bb7c5da76f550aa3d8a1fbff0eb19ccb1a313d55cda56c9ec2ef29632387fe8d76e3c0468043e8f663f4860ee12bf2d5b0b7474d6e694f91e6dcc4024ffffffffffffffff";
const MODP18_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a92108011a723c12a787e6d788719a10bdba5b2699c327186af4e23c1a946834b6150bda2583e9ca2ad44ce8dbbbc2db04de8ef92e8efc141fbecaa6287c59474e6bc05d99b2964fa090c3a2233ba186515be7ed1f612970cee2d7afb81bdd762170481cd0069127d5b05aa993b4ea988d8fddc186ffb7dc90a6c08f4df435c93402849236c3fab4d27c7026c1d4dcb2602646dec9751e763dba37bdf8ff9406ad9e530ee5db382f413001aeb06a53ed9027d831179727b0865a8918da3edbebcf9b14ed44ce6cbaced4bb1bdb7f1447e6cc254b332051512bd7af426fb8f401378cd2bf5983ca01c64b92ecf032ea15d1721d03f482d7ce6e74fef6d55e702f46980c82b5a84031900b1c9e59e7c97fbec7e8f323a97a7e36cc88be0f1d45b7ff585ac54bd407b22b4154aacc8f6d7ebf48e1d814cc5ed20f8037e0a79715eef29be32806a1d58bb7c5da76f550aa3d8a1fbff0eb19ccb1a313d55cda56c9ec2ef29632387fe8d76e3c0468043e8f663f4860ee12bf2d5b0b7474d6e694f91e6dbe115974a3926f12fee5e438777cb6a932df8cd8bec4d073b931ba3bc832b68d9dd300741fa7bf8afc47ed2576f6936ba424663aab639c5ae4f5683423b4742bf1c978238f16cbe39d652de3fdb8befc848ad922222e04a4037c0713eb57a81a23f0c73473fc646cea306b4bcbc8862f8385ddfa9d4b7fa2c087e879683303ed5bdd3a062b3cf5b3a278a66d2a13f83f44f82ddf310ee074ab6a364597e899a0255dc164f31cc50846851df9ab48195ded7ea1b1d510bd7ee74d73faf36bc31ecfa268359046f4eb879f924009438b481c6cd7889a002ed5ee382bc9190da6fc026e479558e4475677e9aa9e3050e2765694dfc81f56e880b96e7160c980dd98edd3dfffffffffffffffff";

const SMALL_PRIMES: &[u32] = &[
    2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97,
    101, 103, 107, 109, 113, 127, 131, 137, 139, 149, 151, 157, 163, 167, 173, 179, 181, 191, 193,
    197, 199, 211, 223, 227, 229, 233, 239, 241, 251, 257, 263, 269, 271, 277, 281, 283, 293,
];

fn random_biguint_below(bound: &BigUint) -> BigUint {
    let bits = bound.bits() as usize;
    if bits == 0 {
        return BigUint::from(0u8);
    }
    let byte_len = bits.div_ceil(8);
    let top_mask = ((1u16 << (bits % 8)) - 1) as u8;
    let mut rng = rand::rng();
    loop {
        let mut bytes = vec![0u8; byte_len];
        rng.fill(bytes.as_mut_slice());
        if !bits.is_multiple_of(8) {
            bytes[0] &= top_mask;
        }
        let candidate = BigUint::from_bytes_be(&bytes);
        if &candidate < bound {
            return candidate;
        }
    }
}

fn is_probable_prime(n: &BigUint, rounds: usize) -> bool {
    let one = BigUint::from(1u8);
    let two = BigUint::from(2u8);
    let three = BigUint::from(3u8);
    if n < &two {
        return false;
    }
    for &sp in SMALL_PRIMES {
        let p = BigUint::from(sp);
        if n == &p {
            return true;
        }
        if n % &p == BigUint::from(0u8) {
            return false;
        }
    }
    if n < &BigUint::from(293u32 * 293u32) {
        return true;
    }

    let n_minus_one = n - &one;
    let mut d = n_minus_one.clone();
    let mut r = 0u32;
    while !d.bit(0) {
        d >>= 1usize;
        r += 1;
    }

    let range = &n_minus_one - &two;
    let mut rng = rand::rng();
    'witness: for _ in 0..rounds {
        let a = if range > one {
            random_biguint_below(&range) + &two
        } else {
            let mut b = [0u8; 4];
            rng.fill(&mut b);
            (BigUint::from(u32::from_be_bytes(b)) % &range) + &two
        };
        if a >= n_minus_one || a < two {
            continue;
        }
        let mut x = a.modpow(&d, n);
        if x == one || x == n_minus_one {
            continue 'witness;
        }
        for _ in 1..r {
            x = (&x * &x) % n;
            if x == n_minus_one {
                continue 'witness;
            }
            if x == one || x == three && n == &three {
                return false;
            }
        }
        return false;
    }
    true
}

fn random_odd_nbit(bits: usize) -> BigUint {
    let byte_len = bits.div_ceil(8);
    let mut bytes = vec![0u8; byte_len];
    rand::rng().fill(bytes.as_mut_slice());
    let top_bit_offset = (bits - 1) % 8;
    let top_mask = ((1u16 << (top_bit_offset + 1)) - 1) as u8;
    bytes[0] &= top_mask;
    bytes[0] |= 1u8 << top_bit_offset;
    bytes[byte_len - 1] |= 1;
    BigUint::from_bytes_be(&bytes)
}

fn generate_prime_internal(
    bits: usize,
    safe: bool,
    add: Option<&BigUint>,
    rem: Option<&BigUint>,
) -> std::result::Result<Vec<u8>, String> {
    if bits < 2 {
        return Err("Prime size must be at least 2 bits".into());
    }
    if safe && bits < 4 {
        return Err("Safe prime size must be at least 4 bits".into());
    }
    let two = BigUint::from(2u8);
    let one = BigUint::from(1u8);
    let expected_bytes = bits.div_ceil(8);

    loop {
        let mut candidate = if safe {
            let q = random_odd_nbit(bits - 1);
            if !is_probable_prime(&q, 16) {
                continue;
            }
            (&q * &two) + &one
        } else {
            random_odd_nbit(bits)
        };

        if let Some(a) = add {
            let target_rem = rem.cloned().unwrap_or_else(|| one.clone()) % a;
            let cur_rem = &candidate % a;
            if cur_rem != target_rem {
                let diff = if target_rem >= cur_rem {
                    &target_rem - &cur_rem
                } else {
                    a - (&cur_rem - &target_rem)
                };
                candidate += diff;
            }
            if candidate.bits() as usize != bits {
                continue;
            }
        }

        if candidate.bits() as usize != bits || !candidate.bit(0) {
            continue;
        }
        if safe {
            let q = (&candidate - &one) >> 1usize;
            if !is_probable_prime(&q, 32) {
                continue;
            }
        }
        if is_probable_prime(&candidate, 40) {
            let mut be = candidate.to_bytes_be();
            if be.len() < expected_bytes {
                let mut padded = vec![0u8; expected_bytes - be.len()];
                padded.extend_from_slice(&be);
                be = padded;
            }
            return Ok(be);
        }
    }
}

fn biguint_from_js<'js>(ctx: &Ctx<'js>, val: Value<'js>, arg_name: &str) -> Result<BigUint> {
    if let Some(obj) = val.as_object() {
        if let Some(bytes) = ObjectBytes::from_array_buffer(obj)? {
            return Ok(BigUint::from_bytes_be(bytes.as_bytes(ctx)?));
        }
    }
    if val.is_big_int() {
        let global = ctx.globals();
        let to_hex: Function = ctx.eval("(v) => v < 0n ? null : v.toString(16)")?;
        let hex_val: Value = to_hex.call((val,))?;
        let Some(hex_js) = hex_val.as_string() else {
            return Err(Exception::throw_range(
                ctx,
                &format!("The \"{arg_name}\" bigint must be non-negative"),
            ));
        };
        let mut hex = hex_js.to_string()?;
        if hex.len() % 2 == 1 {
            hex.insert(0, '0');
        }
        let bytes = lexe_encoding::bytes_from_hex(hex.as_bytes()).or_throw(ctx)?;
        let _ = global;
        return Ok(BigUint::from_bytes_be(&bytes));
    }
    if let Some(n) = val.as_number() {
        if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
            return Err(Exception::throw_range(
                ctx,
                &format!("Invalid integer value for {arg_name}"),
            ));
        }
        return Ok(BigUint::from(n as u64));
    }
    Err(Exception::throw_type(
        ctx,
        &format!("The \"{arg_name}\" argument must be an ArrayBuffer, TypedArray, Buffer, DataView, or bigint"),
    ))
}

fn biguint_to_padded_bytes(n: &BigUint, min_len: usize) -> Vec<u8> {
    let be = n.to_bytes_be();
    if be.len() >= min_len {
        return be;
    }
    let mut padded = vec![0u8; min_len - be.len()];
    padded.extend_from_slice(&be);
    padded
}

fn format_dh_bytes<'js>(
    ctx: &Ctx<'js>,
    bytes: Vec<u8>,
    encoding: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let enc = optional_encoding(ctx, encoding.0)?;
    encode_bytes_to_output(ctx, bytes, enc.as_deref())
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct DiffieHellman {
    #[qjs(skip_trace)]
    prime: BigUint,
    #[qjs(skip_trace)]
    prime_bytes_len: usize,
    #[qjs(skip_trace)]
    generator: BigUint,
    #[qjs(skip_trace)]
    private_key: Option<BigUint>,
    #[qjs(skip_trace)]
    public_key: Option<BigUint>,
    verify_error: i32,
}

impl DiffieHellman {
    fn from_prime_and_gen(
        prime: BigUint,
        prime_bytes_len: usize,
        generator: BigUint,
        known_safe_modp: bool,
    ) -> Self {
        let mut verify_error = 0i32;
        if known_safe_modp {
            // Standard RFC MODP groups are verified safe primes with g = 2.
            verify_error = 0;
        } else {
            let one = BigUint::from(1u8);
            if !is_probable_prime(&prime, 32) {
                verify_error |= DH_CHECK_P_NOT_PRIME;
            } else {
                let q = (&prime - &one) >> 1usize;
                if !is_probable_prime(&q, 32) {
                    verify_error |= DH_CHECK_P_NOT_SAFE_PRIME;
                }
            }
            let two = BigUint::from(2u8);
            if generator < two || generator >= prime {
                verify_error |= DH_NOT_SUITABLE_GENERATOR;
            } else if verify_error == 0 && generator == two {
                let mod24 = &prime % BigUint::from(24u8);
                let rem11 = BigUint::from(11u8);
                let rem23 = BigUint::from(23u8);
                if mod24 != rem11 && mod24 != rem23 {
                    verify_error |= DH_NOT_SUITABLE_GENERATOR;
                }
            } else if generator != two && generator != BigUint::from(5u8) {
                verify_error |= DH_UNABLE_TO_CHECK_GENERATOR;
            }
        }
        Self {
            prime,
            prime_bytes_len,
            generator,
            private_key: None,
            public_key: None,
            verify_error,
        }
    }

    fn compute_pub(&self, priv_key: &BigUint) -> BigUint {
        self.generator.modpow(priv_key, &self.prime)
    }
}

#[rquickjs::methods]
impl DiffieHellman {
    #[qjs(constructor)]
    pub fn constructor<'js>(
        ctx: Ctx<'js>,
        size_or_key: Value<'js>,
        prime_encoding_or_gen: Opt<Value<'js>>,
        generator_arg: Opt<Value<'js>>,
        generator_encoding_arg: Opt<Value<'js>>,
    ) -> Result<Self> {
        build_diffie_hellman(
            ctx,
            size_or_key,
            prime_encoding_or_gen,
            generator_arg,
            generator_encoding_arg,
        )
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "DiffieHellman"
    }

    #[qjs(get, rename = "verifyError")]
    pub fn verify_error(&self) -> i32 {
        self.verify_error
    }

    #[qjs(rename = "generateKeys")]
    pub fn generate_keys<'js>(
        &mut self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let two = BigUint::from(2u8);
        if self.prime <= two {
            return Err(Exception::throw_message(
                &ctx,
                "Invalid Diffie-Hellman prime",
            ));
        }
        let bound = &self.prime - &two;
        let priv_key = random_biguint_below(&bound) + BigUint::from(1u8);
        let pub_key = self.compute_pub(&priv_key);
        let pub_bytes = biguint_to_padded_bytes(&pub_key, self.prime_bytes_len);
        self.private_key = Some(priv_key);
        self.public_key = Some(pub_key);
        format_dh_bytes(&ctx, pub_bytes, encoding)
    }

    #[qjs(rename = "computeSecret")]
    pub fn compute_secret<'js>(
        &self,
        ctx: Ctx<'js>,
        other_public_key: Value<'js>,
        input_encoding: Opt<Value<'js>>,
        output_encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let Some(ref priv_key) = self.private_key else {
            return Err(Exception::throw_message(
                &ctx,
                "Private key must be set before calling computeSecret",
            ));
        };
        let in_enc = optional_encoding(&ctx, input_encoding.0)?;
        let other_bytes = value_to_bytes_with_encoding(
            &ctx,
            &other_public_key,
            in_enc.as_deref(),
            "otherPublicKey",
        )?;
        let other = BigUint::from_bytes_be(&other_bytes);
        let one = BigUint::from(1u8);
        if other <= one || other >= &self.prime - &one {
            return Err(Exception::throw_message(
                &ctx,
                "Supplied key is invalid for Diffie-Hellman computation",
            ));
        }
        let shared = other.modpow(priv_key, &self.prime);
        let shared_bytes = biguint_to_padded_bytes(&shared, self.prime_bytes_len);
        let out_enc = optional_encoding(&ctx, output_encoding.0)?;
        encode_bytes_to_output(&ctx, shared_bytes, out_enc.as_deref())
    }

    #[qjs(rename = "getPrime")]
    pub fn get_prime<'js>(&self, ctx: Ctx<'js>, encoding: Opt<Value<'js>>) -> Result<Value<'js>> {
        let bytes = biguint_to_padded_bytes(&self.prime, self.prime_bytes_len);
        format_dh_bytes(&ctx, bytes, encoding)
    }

    #[qjs(rename = "getGenerator")]
    pub fn get_generator<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let bytes = self.generator.to_bytes_be();
        format_dh_bytes(&ctx, bytes, encoding)
    }

    #[qjs(rename = "getPublicKey")]
    pub fn get_public_key<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let Some(ref pk) = self.public_key else {
            return Err(Exception::throw_message(&ctx, "No public key has been set"));
        };
        let bytes = biguint_to_padded_bytes(pk, self.prime_bytes_len);
        format_dh_bytes(&ctx, bytes, encoding)
    }

    #[qjs(rename = "getPrivateKey")]
    pub fn get_private_key<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let Some(ref sk) = self.private_key else {
            return Err(Exception::throw_message(
                &ctx,
                "No private key has been set",
            ));
        };
        let bytes = biguint_to_padded_bytes(sk, self.prime_bytes_len);
        format_dh_bytes(&ctx, bytes, encoding)
    }

    #[qjs(rename = "setPublicKey")]
    pub fn set_public_key<'js>(
        this: rquickjs::prelude::This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        public_key: Value<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let enc = optional_encoding(&ctx, encoding.0)?;
        let bytes = value_to_bytes_with_encoding(&ctx, &public_key, enc.as_deref(), "publicKey")?;
        this.0.borrow_mut().public_key = Some(BigUint::from_bytes_be(&bytes));
        Ok(this.0)
    }

    #[qjs(rename = "setPrivateKey")]
    pub fn set_private_key<'js>(
        this: rquickjs::prelude::This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        private_key: Value<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let enc = optional_encoding(&ctx, encoding.0)?;
        let bytes = value_to_bytes_with_encoding(&ctx, &private_key, enc.as_deref(), "privateKey")?;
        let sk = BigUint::from_bytes_be(&bytes);
        let mut inner = this.0.borrow_mut();
        if inner.public_key.is_none() {
            inner.public_key = Some(inner.compute_pub(&sk));
        }
        inner.private_key = Some(sk);
        drop(inner);
        Ok(this.0)
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct DiffieHellmanGroup {
    #[qjs(skip_trace)]
    inner: DiffieHellman,
}

#[rquickjs::methods]
impl DiffieHellmanGroup {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'_>, group_name: String) -> Result<Self> {
        build_diffie_hellman_group(&ctx, &group_name)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "DiffieHellmanGroup"
    }

    #[qjs(get, rename = "verifyError")]
    pub fn verify_error(&self) -> i32 {
        self.inner.verify_error
    }

    #[qjs(rename = "generateKeys")]
    pub fn generate_keys<'js>(
        &mut self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        self.inner.generate_keys(ctx, encoding)
    }

    #[qjs(rename = "computeSecret")]
    pub fn compute_secret<'js>(
        &self,
        ctx: Ctx<'js>,
        other_public_key: Value<'js>,
        input_encoding: Opt<Value<'js>>,
        output_encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        self.inner
            .compute_secret(ctx, other_public_key, input_encoding, output_encoding)
    }

    #[qjs(rename = "getPrime")]
    pub fn get_prime<'js>(&self, ctx: Ctx<'js>, encoding: Opt<Value<'js>>) -> Result<Value<'js>> {
        self.inner.get_prime(ctx, encoding)
    }

    #[qjs(rename = "getGenerator")]
    pub fn get_generator<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        self.inner.get_generator(ctx, encoding)
    }

    #[qjs(rename = "getPublicKey")]
    pub fn get_public_key<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        self.inner.get_public_key(ctx, encoding)
    }

    #[qjs(rename = "getPrivateKey")]
    pub fn get_private_key<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        self.inner.get_private_key(ctx, encoding)
    }
}

fn build_diffie_hellman_group(ctx: &Ctx<'_>, group_name: &str) -> Result<DiffieHellmanGroup> {
    let hex = match group_name {
        "modp1" => MODP1_HEX,
        "modp2" => MODP2_HEX,
        "modp5" => MODP5_HEX,
        "modp14" => MODP14_HEX,
        "modp15" => MODP15_HEX,
        "modp16" => MODP16_HEX,
        "modp17" => MODP17_HEX,
        "modp18" => MODP18_HEX,
        _ => {
            return Err(Exception::throw_message(
                ctx,
                &format!("Unknown Diffie-Hellman group: {group_name}"),
            ))
        },
    };
    let prime_bytes = lexe_encoding::bytes_from_hex(hex.as_bytes()).or_throw(ctx)?;
    let prime_len = prime_bytes.len();
    let prime = BigUint::from_bytes_be(&prime_bytes);
    let inner = DiffieHellman::from_prime_and_gen(prime, prime_len, BigUint::from(2u8), true);
    Ok(DiffieHellmanGroup { inner })
}

pub fn create_diffie_hellman_group<'js>(
    ctx: Ctx<'js>,
    group_name: String,
) -> Result<Class<'js, DiffieHellmanGroup>> {
    let group = build_diffie_hellman_group(&ctx, &group_name)?;
    Class::instance(ctx, group)
}

fn build_diffie_hellman<'js>(
    ctx: Ctx<'js>,
    size_or_key: Value<'js>,
    prime_encoding_or_gen: Opt<Value<'js>>,
    generator_arg: Opt<Value<'js>>,
    generator_encoding_arg: Opt<Value<'js>>,
) -> Result<DiffieHellman> {
    if let Some(prime_bits) = size_or_key.as_number() {
        if !prime_bits.is_finite() || prime_bits < 2.0 || prime_bits.fract() != 0.0 {
            return throw_code_error(
                &ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                "primeLength must be an integer >= 2",
            );
        }
        let gen = match prime_encoding_or_gen.0 {
            Some(v) if !v.is_undefined() && !v.is_null() => {
                if let Some(n) = v.as_number() {
                    if !n.is_finite() || n < 2.0 {
                        return Err(Exception::throw_type(&ctx, "Invalid generator"));
                    }
                    BigUint::from(n as u64)
                } else {
                    let g_enc = optional_encoding(&ctx, generator_arg.0)?;
                    let g_bytes =
                        value_to_bytes_with_encoding(&ctx, &v, g_enc.as_deref(), "generator")?;
                    BigUint::from_bytes_be(&g_bytes)
                }
            },
            _ => BigUint::from(2u8),
        };
        let prime_bytes =
            generate_prime_internal(prime_bits as usize, true, None, None).or_throw(&ctx)?;
        let prime_len = prime_bytes.len();
        let prime = BigUint::from_bytes_be(&prime_bytes);
        return Ok(DiffieHellman::from_prime_and_gen(
            prime, prime_len, gen, false,
        ));
    }

    let prime_enc = optional_encoding(&ctx, prime_encoding_or_gen.0)?;
    let prime_bytes =
        value_to_bytes_with_encoding(&ctx, &size_or_key, prime_enc.as_deref(), "prime")?;
    let prime_len = prime_bytes.len();
    let prime = BigUint::from_bytes_be(&prime_bytes);

    let gen = match generator_arg.0 {
        Some(v) if !v.is_undefined() && !v.is_null() => {
            if let Some(n) = v.as_number() {
                BigUint::from(n as u64)
            } else {
                let g_enc = optional_encoding(&ctx, generator_encoding_arg.0)?;
                let g_bytes =
                    value_to_bytes_with_encoding(&ctx, &v, g_enc.as_deref(), "generator")?;
                BigUint::from_bytes_be(&g_bytes)
            }
        },
        _ => BigUint::from(2u8),
    };

    Ok(DiffieHellman::from_prime_and_gen(
        prime, prime_len, gen, false,
    ))
}

pub fn create_diffie_hellman<'js>(
    ctx: Ctx<'js>,
    size_or_key: Value<'js>,
    prime_encoding_or_gen: Opt<Value<'js>>,
    generator_arg: Opt<Value<'js>>,
    generator_encoding_arg: Opt<Value<'js>>,
) -> Result<Class<'js, DiffieHellman>> {
    let dh = build_diffie_hellman(
        ctx.clone(),
        size_or_key,
        prime_encoding_or_gen,
        generator_arg,
        generator_encoding_arg,
    )?;
    Class::instance(ctx, dh)
}

pub fn check_prime_sync<'js>(
    ctx: Ctx<'js>,
    candidate: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<bool> {
    let n = biguint_from_js(&ctx, candidate, "candidate")?;
    let checks = match options.0 {
        Some(v) if !v.is_undefined() && !v.is_null() => {
            let obj = v.into_object().ok_or_else(|| {
                Exception::throw_type(&ctx, "The \"options\" argument must be of type object")
            })?;
            match obj.get::<_, Value>("checks") {
                Ok(c) if !c.is_undefined() && !c.is_null() => {
                    let Some(num) = c.as_number() else {
                        return Err(Exception::throw_type(
                            &ctx,
                            "The \"options.checks\" property must be of type number",
                        ));
                    };
                    if !num.is_finite() || num < 0.0 || num.fract() != 0.0 {
                        return throw_code_error(
                            &ctx,
                            "RangeError",
                            "ERR_OUT_OF_RANGE",
                            "The value of \"options.checks\" is out of range",
                        );
                    }
                    (num as usize).max(1)
                },
                _ => 64,
            }
        },
        _ => 64,
    };
    Ok(is_probable_prime(&n, checks))
}

pub fn check_prime<'js>(
    ctx: Ctx<'js>,
    candidate: Value<'js>,
    options_or_cb: Value<'js>,
    cb_opt: Opt<Function<'js>>,
) -> Result<()> {
    let (options, callback) = if let Some(cb) = cb_opt.0 {
        (Some(options_or_cb), cb)
    } else {
        let cb = options_or_cb.into_function().ok_or_else(|| {
            Exception::throw_type(&ctx, "The \"callback\" argument must be of type function")
        })?;
        (None, cb)
    };
    let n = biguint_from_js(&ctx, candidate, "candidate")?;
    let checks = match options {
        Some(v) if !v.is_undefined() && !v.is_null() => {
            let obj = v.into_object().ok_or_else(|| {
                Exception::throw_type(&ctx, "The \"options\" argument must be of type object")
            })?;
            match obj.get::<_, Value>("checks") {
                Ok(c) if !c.is_undefined() && !c.is_null() => {
                    c.as_number().unwrap_or(64.0).max(1.0) as usize
                },
                _ => 64,
            }
        },
        _ => 64,
    };
    ctx.clone().spawn_exit(async move {
        let res = tokio::task::spawn_blocking(move || is_probable_prime(&n, checks))
            .await
            .map_err(|e| format!("checkPrime task panicked: {e}"));
        match res {
            Ok(is_prime) => {
                let _: () = callback.call((Null.into_js(&ctx), is_prime))?;
            },
            Err(msg) => {
                let err = Exception::throw_message(&ctx, &msg).into_value(&ctx)?;
                let _: () = callback.call((err,))?;
            },
        }
        Ok::<_, rquickjs::Error>(())
    })?;
    Ok(())
}

struct GeneratePrimeOpts {
    bits: usize,
    safe: bool,
    add: Option<BigUint>,
    rem: Option<BigUint>,
    as_bigint: bool,
}

fn parse_generate_prime_args<'js>(
    ctx: &Ctx<'js>,
    size: i64,
    options: Option<Value<'js>>,
) -> Result<GeneratePrimeOpts> {
    if !(2..=16384).contains(&size) {
        return throw_code_error(
            ctx,
            "RangeError",
            "ERR_OUT_OF_RANGE",
            "The value of \"size\" is out of range",
        );
    }
    let mut safe = false;
    let mut add = None;
    let mut rem = None;
    let mut as_bigint = false;
    if let Some(opt_val) = options {
        if !opt_val.is_undefined() && !opt_val.is_null() {
            let obj = opt_val.into_object().ok_or_else(|| {
                Exception::throw_type(ctx, "The \"options\" argument must be of type object")
            })?;
            if let Ok(v) = obj.get::<_, Value>("safe") {
                if let Some(b) = v.as_bool() {
                    safe = b;
                }
            }
            if let Ok(v) = obj.get::<_, Value>("bigint") {
                if let Some(b) = v.as_bool() {
                    as_bigint = b;
                }
            }
            if let Ok(v) = obj.get::<_, Value>("add") {
                if !v.is_undefined() && !v.is_null() {
                    add = Some(biguint_from_js(ctx, v, "options.add")?);
                }
            }
            if let Ok(v) = obj.get::<_, Value>("rem") {
                if !v.is_undefined() && !v.is_null() {
                    rem = Some(biguint_from_js(ctx, v, "options.rem")?);
                }
            }
        }
    }
    Ok(GeneratePrimeOpts {
        bits: size as usize,
        safe,
        add,
        rem,
        as_bigint,
    })
}

fn prime_bytes_to_js<'js>(ctx: &Ctx<'js>, bytes: Vec<u8>, as_bigint: bool) -> Result<Value<'js>> {
    if as_bigint {
        let hex = bytes_to_hex_string(&bytes);
        let bigint_fn: Function = ctx.globals().get("BigInt")?;
        return bigint_fn.call((format!("0x{hex}"),));
    }
    ArrayBuffer::new(ctx.clone(), bytes)?.into_js(ctx)
}

pub fn generate_prime_sync<'js>(
    ctx: Ctx<'js>,
    size: i64,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let opts = parse_generate_prime_args(&ctx, size, options.0)?;
    let bytes = generate_prime_internal(opts.bits, opts.safe, opts.add.as_ref(), opts.rem.as_ref())
        .or_throw(&ctx)?;
    prime_bytes_to_js(&ctx, bytes, opts.as_bigint)
}

pub fn generate_prime<'js>(
    ctx: Ctx<'js>,
    size: i64,
    options_or_cb: Value<'js>,
    cb_opt: Opt<Function<'js>>,
) -> Result<()> {
    let (options, callback) = if let Some(cb) = cb_opt.0 {
        (Some(options_or_cb), cb)
    } else {
        let cb = options_or_cb.into_function().ok_or_else(|| {
            Exception::throw_type(&ctx, "The \"callback\" argument must be of type function")
        })?;
        (None, cb)
    };
    let opts = parse_generate_prime_args(&ctx, size, options)?;
    let as_bigint = opts.as_bigint;
    ctx.clone().spawn_exit(async move {
        let res = tokio::task::spawn_blocking(move || {
            generate_prime_internal(opts.bits, opts.safe, opts.add.as_ref(), opts.rem.as_ref())
        })
        .await
        .map_err(|e| format!("generatePrime task panicked: {e}"))
        .and_then(|r| r);
        match res {
            Ok(bytes) => {
                let js_val = prime_bytes_to_js(&ctx, bytes, as_bigint)?;
                let _: () = callback.call((Null.into_js(&ctx), js_val))?;
            },
            Err(msg) => {
                let err = Exception::throw_message(&ctx, &msg).into_value(&ctx)?;
                let _: () = callback.call((err,))?;
            },
        }
        Ok::<_, rquickjs::Error>(())
    })?;
    Ok(())
}
