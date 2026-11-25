pub use ::core::ffi::{c_char, c_int, c_uint, c_void};

pub type JSValueConst = JSValue;

pub const JS_NULL: JSValue = JS_MKVAL(JS_TAG_NULL, 0);
pub const JS_UNDEFINED: JSValue = JS_MKVAL(JS_TAG_UNDEFINED, 0);
pub const JS_FALSE: JSValue = JS_MKVAL(JS_TAG_BOOL, 0);
pub const JS_TRUE: JSValue = JS_MKVAL(JS_TAG_BOOL, 1);
pub const JS_EXCEPTION: JSValue = JS_MKVAL(JS_TAG_EXCEPTION, 0);
pub const JS_UNINITIALIZED: JSValue = JS_MKVAL(JS_TAG_UNINITIALIZED, 0);

#[inline]
pub unsafe fn JS_VALUE_HAS_REF_COUNT(v: JSValue) -> bool {
    JS_VALUE_GET_TAG(v) as c_uint >= JS_TAG_FIRST as c_uint
}

#[inline]
pub unsafe fn JS_IsNumber(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_INT || JS_TAG_IS_FLOAT64(tag)
}

#[inline]
pub unsafe fn JS_IsInt(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_INT
}

#[inline]
pub unsafe fn JS_IsBigInt(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_BIG_INT || tag == JS_TAG_SHORT_BIG_INT
}

#[inline]
pub unsafe fn JS_IsBool(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_BOOL
}

#[inline]
pub unsafe fn JS_IsNull(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_NULL
}

#[inline]
pub unsafe fn JS_IsUndefined(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_UNDEFINED
}

#[inline]
pub unsafe fn JS_IsException(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_EXCEPTION
}

#[inline]
pub unsafe fn JS_IsUninitialized(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_UNINITIALIZED
}

#[inline]
pub unsafe fn JS_IsString(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_STRING || tag == JS_TAG_STRING_ROPE
}

#[inline]
pub unsafe fn JS_IsSymbol(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_SYMBOL
}

#[inline]
pub unsafe fn JS_IsObject(v: JSValue) -> bool {
    let tag = JS_VALUE_GET_TAG(v);
    tag == JS_TAG_OBJECT
}

#[inline]
pub unsafe fn JS_ToCString(ctx: *mut JSContext, val: JSValue) -> *const c_char {
    // Type her can differ depending on architecture.
    #[allow(clippy::useless_conversion)]
    JS_ToCStringLen2(ctx, ptr::null_mut(), val, (false).into())
}
#[inline]
pub unsafe fn JS_ToCStringLen(
    ctx: *mut JSContext,
    plen: *mut usize,
    val: JSValue,
) -> *const c_char {
    // Type her can differ depending on architecture.
    #[allow(clippy::useless_conversion)]
    JS_ToCStringLen2(ctx, plen as _, val, (false).into())
}

#[inline]
pub fn JS_NewFloat64(d: f64) -> JSValue {
    union U {
        d: f64,
        u: u64,
    }

    let u = U { d };
    let val = d as i32;
    let t = U { d: val as f64 };
    /* -0 cannot be represented as integer, so we compare the bit
    representation */
    if unsafe { u.u == t.u } {
        JS_MKVAL(JS_TAG_INT, val)
    } else {
        __JS_NewFloat64(d)
    }
}

#[cfg(not(feature = "bindgen"))]
pub const JS_EVAL_FLAG_NO_RESOLVE: c_int = 1 << 8;

#[cfg(not(feature = "bindgen"))]
pub const JS_READ_OBJ_NO_RESOLVE: c_int = 1 << 4;

#[cfg(not(feature = "bindgen"))]
pub type JSGCHook = ::core::option::Option<
    unsafe extern "C" fn(rt: *mut JSRuntime, phase: c_int, flags: c_int, opaque: *mut c_void),
>;

#[cfg(not(feature = "bindgen"))]
pub type JSDynamicImportHook = ::core::option::Option<
    unsafe extern "C" fn(
        ctx: *mut JSContext,
        basename: JSValueConst,
        specifier: JSValueConst,
        attributes: JSValueConst,
        opaque: *mut c_void,
    ) -> JSValue,
>;

#[cfg(not(feature = "bindgen"))]
extern "C" {
    pub fn JS_SetGCHook(rt: *mut JSRuntime, gc_hook: JSGCHook, opaque: *mut c_void);
    pub fn JS_SetDynamicImportHook(
        rt: *mut JSRuntime,
        cb: JSDynamicImportHook,
        opaque: *mut c_void,
    );
    pub fn JS_SetCodegenFromStrings(ctx: *mut JSContext, allow: bool);
    pub fn JS_SetGlobalObject(ctx: *mut JSContext, global_obj: JSValue);
    pub fn JS_ExecutePendingJobForContext(rt: *mut JSRuntime, target_ctx: *mut JSContext) -> c_int;
    pub fn JS_ValueToModule(val: JSValueConst) -> *mut JSModuleDef;
    pub fn JS_ModuleToValue(ctx: *mut JSContext, m: *mut JSModuleDef) -> JSValue;
    pub fn JS_GetModuleReqCount(m: *mut JSModuleDef) -> c_int;
    pub fn JS_GetModuleReqName(ctx: *mut JSContext, m: *mut JSModuleDef, idx: c_int) -> JSValue;
    pub fn JS_GetModuleReqAttributes(
        ctx: *mut JSContext,
        m: *mut JSModuleDef,
        idx: c_int,
    ) -> JSValue;
    pub fn JS_SetModuleReqModule(
        m: *mut JSModuleDef,
        idx: c_int,
        dep: *mut JSModuleDef,
    ) -> c_int;
    pub fn JS_SetModuleResolved(m: *mut JSModuleDef, resolved: bool);
    pub fn JS_VmLinkModule(ctx: *mut JSContext, m: *mut JSModuleDef) -> c_int;
    pub fn JS_VmEvaluateModule(ctx: *mut JSContext, m: *mut JSModuleDef) -> JSValue;
    pub fn JS_GetModuleStatus(m: *mut JSModuleDef) -> c_int;
    pub fn JS_GetModuleEvalException(ctx: *mut JSContext, m: *mut JSModuleDef) -> JSValue;
    pub fn JS_ModuleHasTLA(m: *mut JSModuleDef) -> bool;
    pub fn JS_IsNonDetachableArrayBuffer(obj: JSValueConst) -> c_int;
    pub fn JS_SetNonDetachableArrayBuffer(obj: JSValueConst, non_detachable: bool) -> c_int;
    pub fn JS_IsSharedArrayBuffer(obj: JSValueConst) -> bool;
}
