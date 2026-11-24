// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashSet,
    ffi::{CStr, CString},
    mem::MaybeUninit,
    ptr::NonNull,
};

use rquickjs::{qjs, Ctx, Error, Exception, Object, Result as JsResult, Value};

const DEFAULT_FILENAME: &str = "evalmachine.<anonymous>";
const GPN_FLAGS: i32 = (qjs::JS_GPN_STRING_MASK | qjs::JS_GPN_ENUM_ONLY) as i32;

fn ctx_ptr(ctx: &Ctx<'_>) -> *mut qjs::JSContext {
    ctx.as_raw().as_ptr()
}

fn is_exception(value: qjs::JSValue) -> bool {
    unsafe { qjs::JS_VALUE_GET_NORM_TAG(value) == qjs::JS_TAG_EXCEPTION }
}

/// Temporary QuickJS context created with `JS_NewContext` on the same runtime as `parent`.
///
/// Parent and child contexts must belong to the same runtime. Values transferred between them
/// must be reference-counted with `JS_DupValue`. Before the child context is destroyed, ownership
/// of return values and exceptions must be transferred out. This type must not escape the module.
struct ChildContext {
    ptr: NonNull<qjs::JSContext>,
}

impl ChildContext {
    /// # Safety
    /// `parent` must be a valid context handle. The parent and child will share one runtime.
    unsafe fn new(parent: &Ctx<'_>) -> JsResult<Self> {
        let rt = qjs::JS_GetRuntime(ctx_ptr(parent));
        let ptr = qjs::JS_NewContext(rt);
        NonNull::new(ptr)
            .map(|ptr| Self { ptr })
            .ok_or(Error::Allocation)
    }

    fn as_ptr(&self) -> *mut qjs::JSContext {
        self.ptr.as_ptr()
    }
}

impl Drop for ChildContext {
    fn drop(&mut self) {
        unsafe { qjs::JS_FreeContext(self.as_ptr()) };
    }
}

/// RAII guard for JSValues owned by a child context.
struct ChildValue {
    ctx: *mut qjs::JSContext,
    value: qjs::JSValue,
}

impl ChildValue {
    fn new(ctx: *mut qjs::JSContext, value: qjs::JSValue) -> Self {
        Self { ctx, value }
    }

    fn transfer_to_parent(self, parent: Ctx<'_>) -> Value<'_> {
        let child_ctx = self.ctx;
        let value = self.value;
        std::mem::forget(self);
        let copied = unsafe { qjs::JS_DupValue(ctx_ptr(&parent), value) };
        unsafe { qjs::JS_FreeValue(child_ctx, value) };
        unsafe { Value::from_raw(parent, copied) }
    }
}

impl Drop for ChildValue {
    fn drop(&mut self) {
        unsafe { qjs::JS_FreeValue(self.ctx, self.value) };
    }
}

struct ChildScope<'a, 'js> {
    parent_ctx: &'a Ctx<'js>,
    child: &'a ChildContext,
}

impl ChildScope<'_, '_> {
    /// # Safety
    /// The child context must have a pending exception.
    unsafe fn capture_exception(&self) -> ChildValue {
        let exception = qjs::JS_GetException(self.child.as_ptr());
        ChildValue::new(self.child.as_ptr(), exception)
    }

    /// # Safety
    /// `value` must be the result of a child-context operation.
    unsafe fn check_jsvalue(&self, value: qjs::JSValue) -> ChildResult<qjs::JSValue> {
        if is_exception(value) {
            qjs::JS_FreeValue(self.child.as_ptr(), value);
            Err(self.capture_exception())
        } else {
            Ok(value)
        }
    }

    /// # Safety
    /// `status` must be the result of a child-context operation.
    unsafe fn check_status(&self, status: i32) -> ChildResult<()> {
        if status < 0 {
            Err(self.capture_exception())
        } else {
            Ok(())
        }
    }
}

type ChildResult<T> = std::result::Result<T, ChildValue>;

fn child_exception_to_error<'js>(parent: Ctx<'js>, exception: ChildValue) -> Error {
    let transferred = exception.transfer_to_parent(parent.clone());
    parent.throw(transferred)
}

fn type_error(ctx: &Ctx<'_>, message: &str) -> Error {
    Exception::throw_type(ctx, message)
}

fn unsupported_option(ctx: &Ctx<'_>, name: &str) -> Error {
    Exception::throw_message(
        ctx,
        &format!("vm.runInNewContext option '{name}' is not supported"),
    )
}

fn normalize_optional_value<'js>(value: Option<Value<'js>>) -> Option<Value<'js>> {
    value.filter(|value| !value.is_undefined())
}

fn value_to_string<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> JsResult<String> {
    let str_val = unsafe { qjs::JS_ToString(ctx_ptr(ctx), value.as_raw()) };
    if is_exception(str_val) {
        unsafe {
            qjs::JS_FreeValue(ctx_ptr(ctx), str_val);
            let exc = qjs::JS_GetException(ctx_ptr(ctx));
            return Err(ctx.throw(Value::from_raw(ctx.clone(), exc)));
        }
    }

    let mut len = MaybeUninit::uninit();
    let ptr = unsafe { qjs::JS_ToCStringLen(ctx_ptr(ctx), len.as_mut_ptr(), str_val) };
    unsafe { qjs::JS_FreeValue(ctx_ptr(ctx), str_val) };
    if ptr.is_null() {
        return Err(Error::Unknown);
    }

    let len = unsafe { len.assume_init() };
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as _) };
    let string = String::from_utf8_lossy(bytes).into_owned();
    unsafe { qjs::JS_FreeCString(ctx_ptr(ctx), ptr) };
    Ok(string)
}

fn validated_filename_cstring(ctx: &Ctx<'_>, filename: &str) -> JsResult<CString> {
    if filename.as_bytes().contains(&0) {
        return Err(type_error(
            ctx,
            "The \"options.filename\" property must not contain null bytes",
        ));
    }

    CString::new(filename).map_err(|_| Error::Unknown)
}

fn parse_options(ctx: &Ctx<'_>, options: Option<Value<'_>>) -> JsResult<CString> {
    let filename = match normalize_optional_value(options) {
        None => DEFAULT_FILENAME.to_string(),
        Some(options) if options.is_string() => options.get()?,
        Some(options) => {
            if !options.is_object() || options.is_null() {
                return Err(type_error(
                    ctx,
                    "The \"options\" argument must be a string or object",
                ));
            }

            let object = options.as_object().expect("options is object");
            let mut filename = DEFAULT_FILENAME.to_string();

            for key in object.keys::<String>() {
                let key = key?;
                if key == "filename" {
                    let value: Value = object.get(key.as_str())?;
                    if value.is_undefined() {
                        continue;
                    }
                    if !value.is_string() {
                        return Err(type_error(
                            ctx,
                            "The \"options.filename\" property must be of type string",
                        ));
                    }
                    filename = value.get()?;
                } else {
                    return Err(unsupported_option(ctx, &key));
                }
            }

            filename
        },
    };

    validated_filename_cstring(ctx, &filename)
}

fn as_sandbox_object<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> JsResult<Object<'js>> {
    if value.is_null() || !value.is_object() {
        return Err(type_error(
            ctx,
            "The \"contextObject\" argument must be an object",
        ));
    }

    if value.is_function() {
        return Err(type_error(
            ctx,
            "The \"contextObject\" argument must be an object",
        ));
    }

    if value.is_array() {
        Ok(value.as_array().expect("array").as_object().clone())
    } else {
        Ok(value.as_object().expect("object").clone())
    }
}

fn get_own_enumerable_string_keys(
    scope: &ChildScope<'_, '_>,
    object: qjs::JSValue,
) -> JsResult<Vec<String>> {
    let parent = scope.parent_ctx.clone();
    let mut enums = MaybeUninit::uninit();
    let mut count = MaybeUninit::uninit();
    let child = scope.child.as_ptr();

    let count = unsafe {
        if qjs::JS_GetOwnPropertyNames(
            child,
            enums.as_mut_ptr(),
            count.as_mut_ptr(),
            object,
            GPN_FLAGS,
        ) < 0
        {
            return Err(child_exception_to_error(
                parent.clone(),
                scope.capture_exception(),
            ));
        }
        count.assume_init()
    };
    let enums = unsafe { enums.assume_init() };

    let mut keys = Vec::with_capacity(count as _);
    for index in 0..count {
        let elem = unsafe { &*enums.offset(index as _) };
        let atom = elem.atom;
        keys.push(atom_to_string(scope, atom)?);
        unsafe { qjs::JS_FreeAtom(child, atom) };
    }

    unsafe { qjs::js_free(child, enums as _) };
    Ok(keys)
}

fn atom_to_string(scope: &ChildScope<'_, '_>, atom: qjs::JSAtom) -> JsResult<String> {
    let parent = scope.parent_ctx.clone();
    let child = scope.child.as_ptr();
    let str_val = unsafe { qjs::JS_AtomToString(child, atom) };
    let str_val = unsafe {
        scope
            .check_jsvalue(str_val)
            .map_err(|exception| child_exception_to_error(parent.clone(), exception))?
    };

    let mut len = MaybeUninit::uninit();
    let ptr = unsafe { qjs::JS_ToCStringLen(child, len.as_mut_ptr(), str_val) };
    unsafe { qjs::JS_FreeValue(child, str_val) };
    if ptr.is_null() {
        return Err(Error::Unknown);
    }

    let len = unsafe { len.assume_init() };
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as _) };
    let key = String::from_utf8_lossy(bytes).into_owned();
    unsafe { qjs::JS_FreeCString(child, ptr) };
    Ok(key)
}

/// # Safety
/// `value` ownership is transferred to the child global object.
unsafe fn set_property(
    scope: &ChildScope<'_, '_>,
    object: qjs::JSValue,
    key: &str,
    value: qjs::JSValue,
) -> JsResult<()> {
    let parent = scope.parent_ctx.clone();
    let child = scope.child.as_ptr();
    let key = CString::new(key).map_err(|_| Error::Unknown)?;
    let atom = qjs::JS_NewAtom(child, key.as_ptr());
    let result = qjs::JS_SetProperty(child, object, atom, value);
    qjs::JS_FreeAtom(child, atom);
    scope
        .check_status(result)
        .map_err(|exception| child_exception_to_error(parent, exception))
}

/// # Safety
/// Caller owns the returned value in the child context until transferred or freed.
unsafe fn get_property(
    scope: &ChildScope<'_, '_>,
    object: qjs::JSValue,
    key: &str,
) -> JsResult<ChildValue> {
    let parent = scope.parent_ctx.clone();
    let child = scope.child.as_ptr();
    let key = CString::new(key).map_err(|_| Error::Unknown)?;
    let atom = qjs::JS_NewAtom(child, key.as_ptr());
    let value = qjs::JS_GetProperty(child, object, atom);
    qjs::JS_FreeAtom(child, atom);
    let value = scope
        .check_jsvalue(value)
        .map_err(|exception| child_exception_to_error(parent.clone(), exception))?;
    Ok(ChildValue::new(child, value))
}

struct ChildGlobal {
    ctx: *mut qjs::JSContext,
    value: qjs::JSValue,
}

impl ChildGlobal {
    fn new(child: &ChildContext) -> Self {
        let value = unsafe { qjs::JS_GetGlobalObject(child.as_ptr()) };
        Self {
            ctx: child.as_ptr(),
            value,
        }
    }

    fn raw(&self) -> qjs::JSValue {
        self.value
    }
}

impl Drop for ChildGlobal {
    fn drop(&mut self) {
        unsafe { qjs::JS_FreeValue(self.ctx, self.value) };
    }
}

unsafe fn copy_sandbox_to_child_global(
    scope: &ChildScope<'_, '_>,
    sandbox: &Object<'_>,
    child_global: qjs::JSValue,
) -> JsResult<()> {
    for key in sandbox.keys::<String>() {
        let key = key?;
        let value: Value = sandbox.get(key.as_str())?;
        let copied = qjs::JS_DupValue(ctx_ptr(scope.parent_ctx), value.as_raw());
        set_property(scope, child_global, &key, copied)?;
    }
    Ok(())
}

unsafe fn sync_sandbox_from_child_global<'js>(
    scope: &ChildScope<'_, 'js>,
    sandbox: &Object<'js>,
    child_global: qjs::JSValue,
    initial_global_keys: &HashSet<String>,
    sandbox_original_keys: &HashSet<String>,
) -> JsResult<()> {
    let parent = scope.parent_ctx.clone();
    let current_keys = get_own_enumerable_string_keys(scope, child_global)?;
    let current_key_set: HashSet<String> = current_keys.iter().cloned().collect();

    for key in sandbox_original_keys {
        if current_key_set.contains(key) {
            let value = get_property(scope, child_global, key)?;
            sandbox.set(key.as_str(), value.transfer_to_parent(parent.clone()))?;
        } else {
            sandbox.remove(key.as_str())?;
        }
    }

    for key in current_keys {
        if !initial_global_keys.contains(&key) && !sandbox_original_keys.contains(&key) {
            let value = get_property(scope, child_global, &key)?;
            sandbox.set(key.as_str(), value.transfer_to_parent(parent.clone()))?;
        }
    }

    Ok(())
}

unsafe fn eval_in_child(
    scope: &ChildScope<'_, '_>,
    code: &[u8],
    filename: &CStr,
) -> ChildResult<ChildValue> {
    let child = scope.child.as_ptr();
    let flag = qjs::JS_EVAL_TYPE_GLOBAL as i32;

    let mut source = code.to_vec();
    source.push(0);

    let value = qjs::JS_Eval(
        child,
        source.as_ptr() as *const std::ffi::c_char,
        code.len() as u64,
        filename.as_ptr(),
        flag,
    );

    if is_exception(value) || qjs::JS_HasException(child) {
        qjs::JS_FreeValue(child, value);
        Err(scope.capture_exception())
    } else {
        Ok(ChildValue::new(child, value))
    }
}

pub(crate) fn run_in_new_context_impl<'js>(
    parent_ctx: Ctx<'js>,
    code: Value<'js>,
    context_object: Option<Value<'js>>,
    options: Option<Value<'js>>,
) -> JsResult<Value<'js>> {
    let context_object = normalize_optional_value(context_object);
    let code = value_to_string(&parent_ctx, code)?;
    let filename = parse_options(&parent_ctx, options)?;

    let sync_back = context_object.is_some();
    let sandbox = match context_object {
        Some(value) => as_sandbox_object(&parent_ctx, value)?,
        None => Object::new(parent_ctx.clone())?,
    };

    let sandbox_original_keys: HashSet<String> =
        sandbox.keys::<String>().collect::<JsResult<_>>()?;

    // Parent and child contexts must belong to the same runtime.
    let child = unsafe { ChildContext::new(&parent_ctx)? };
    let child_global = ChildGlobal::new(&child);
    let scope = ChildScope {
        parent_ctx: &parent_ctx,
        child: &child,
    };
    let initial_global_keys: HashSet<String> =
        get_own_enumerable_string_keys(&scope, child_global.raw())?
            .into_iter()
            .collect();

    unsafe {
        copy_sandbox_to_child_global(&scope, &sandbox, child_global.raw())?;
    }
    let eval_result = unsafe { eval_in_child(&scope, code.as_bytes(), filename.as_c_str()) };

    let sync_error = if sync_back {
        unsafe {
            sync_sandbox_from_child_global(
                &scope,
                &sandbox,
                child_global.raw(),
                &initial_global_keys,
                &sandbox_original_keys,
            )
        }
        .err()
    } else {
        None
    };

    match eval_result {
        Err(eval_exception) => Err(child_exception_to_error(parent_ctx, eval_exception)),
        Ok(value) => {
            if let Some(sync_err) = sync_error {
                return Err(sync_err);
            }
            Ok(value.transfer_to_parent(parent_ctx))
        },
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::test_async_with;
    use rquickjs::{CatchResultExt, CaughtError, IntoJs, Object};

    use super::*;

    #[tokio::test]
    async fn returns_last_expression_and_syncs_sandbox() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let sandbox = Object::new(ctx.clone()).unwrap();
                sandbox.set("count", 0).unwrap();

                let sandbox_value: Value = sandbox.clone().into_js(&ctx).unwrap();
                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "count += 1; name = 'lexe'; count".into_js(&ctx).unwrap(),
                    Some(sandbox_value),
                    None,
                )
                .catch(&ctx);

                if let Err(CaughtError::Error(err)) = &result {
                    panic!("runInNewContext failed: {err:?}");
                }
                let result = result.unwrap();
                assert_eq!(result.as_int(), Some(1));
                assert_eq!(sandbox.get::<_, i32>("count").unwrap(), 1);
                assert_eq!(
                    sandbox.get::<_, String>("name").unwrap(),
                    "lexe".to_string()
                );
            })
        })
        .await;
    }

    #[tokio::test]
    async fn sync_getter_exception_is_transferred() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let sandbox = Object::new(ctx.clone()).unwrap();
                let sandbox_value: Value = sandbox.clone().into_js(&ctx).unwrap();
                let result = run_in_new_context_impl(
                    ctx.clone(),
                    r#"Object.defineProperty(globalThis, "boom", { enumerable: true, get() { throw new Error("getter failure"); } })"#
                        .into_js(&ctx)
                        .unwrap(),
                    Some(sandbox_value),
                    None,
                )
                .catch(&ctx);

                assert!(result.is_err(), "expected runInNewContext to fail");
                match result {
                    Err(CaughtError::Value(value)) => {
                        let message = value
                            .as_object()
                            .and_then(|object| object.get::<_, String>("message").ok())
                            .unwrap_or_default();
                        assert!(
                            message.contains("getter failure"),
                            "unexpected error message: {message}"
                        );
                    }
                    Err(CaughtError::Error(err)) => {
                        assert!(
                            err.to_string().contains("getter failure"),
                            "unexpected error: {err:?}"
                        );
                    }
                    Err(CaughtError::Exception(exception)) => {
                        let message = exception.message().unwrap_or_default();
                        assert!(
                            message.contains("getter failure"),
                            "unexpected error message: {message}"
                        );
                    }
                    Ok(_) => panic!("expected error"),
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn preserves_sandbox_object_identity() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let inner = Object::new(ctx.clone()).unwrap();
                inner.set("bar", 1).unwrap();
                let sandbox = Object::new(ctx.clone()).unwrap();
                sandbox.set("inner", inner.clone()).unwrap();

                let sandbox_value: Value = sandbox.clone().into_js(&ctx).unwrap();
                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "inner.bar = 2; inner".into_js(&ctx).unwrap(),
                    Some(sandbox_value),
                    None,
                )
                .catch(&ctx)
                .unwrap();

                assert_eq!(inner.get::<_, i32>("bar").unwrap(), 2);
                assert_eq!(
                    sandbox.get::<_, Value>("inner").unwrap(),
                    inner.into_value()
                );
                assert_eq!(result, sandbox.get::<_, Value>("inner").unwrap());
            })
        })
        .await;
    }

    #[tokio::test]
    async fn prefers_script_exceptions_over_sync_failures() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let sandbox = ctx.eval::<Object, _>("Object.freeze({})").unwrap();
                let sandbox_value: Value = sandbox.clone().into_js(&ctx).unwrap();
                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "var x = 1; throw new Error('script boom')"
                        .into_js(&ctx)
                        .unwrap(),
                    Some(sandbox_value),
                    None,
                )
                .catch(&ctx);

                assert!(result.is_err());
                match result {
                    Err(CaughtError::Value(value)) => {
                        let message = value
                            .as_object()
                            .and_then(|object| object.get::<_, String>("message").ok())
                            .unwrap_or_default();
                        assert!(message.contains("script boom"), "unexpected: {message}");
                    },
                    Err(CaughtError::Error(err)) => {
                        assert!(
                            err.to_string().contains("script boom"),
                            "unexpected: {err:?}"
                        );
                    },
                    Err(CaughtError::Exception(exception)) => {
                        let message = exception.message().unwrap_or_default();
                        assert!(message.contains("script boom"), "unexpected: {message}");
                    },
                    Ok(_) => panic!("expected error"),
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn returned_child_closure_reads_child_global_bindings() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "globalThis.x = 42; () => x".into_js(&ctx).unwrap(),
                    None,
                    None,
                )
                .catch(&ctx)
                .unwrap();
                let function: rquickjs::Function = result.into_function().unwrap();
                assert_eq!(function.call::<_, i32>(()).unwrap(), 42);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn treats_explicit_undefined_context_as_omitted() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "1".into_js(&ctx).unwrap(),
                    Some(Value::new_undefined(ctx.clone())),
                    None,
                )
                .catch(&ctx)
                .unwrap();

                assert_eq!(result.as_int(), Some(1));
            })
        })
        .await;
    }

    #[tokio::test]
    async fn allows_nul_bytes_in_comments_and_string_literals() {
        test_async_with(|ctx| {
            Box::pin(async move {
                for code in ["/*\0*/ 1", "\"a\\0b\""] {
                    let result = run_in_new_context_impl(
                        ctx.clone(),
                        code.into_js(&ctx).unwrap(),
                        None,
                        None,
                    )
                    .catch(&ctx)
                    .unwrap();

                    if code.ends_with("1") {
                        assert_eq!(result.as_int(), Some(1));
                    } else {
                        assert_eq!(result.as_string().unwrap().to_string().unwrap(), "a\u{0}b");
                    }
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_nul_bytes_in_source_with_syntax_error() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let code = ctx.eval::<String, _>("'1\\u0000'").unwrap();
                let result =
                    run_in_new_context_impl(ctx.clone(), code.into_js(&ctx).unwrap(), None, None)
                        .catch(&ctx);

                assert!(result.is_err());
                match result {
                    Err(CaughtError::Value(value)) => {
                        let name = value
                            .as_object()
                            .and_then(|object| object.get::<_, String>("name").ok())
                            .unwrap_or_default();
                        assert_eq!(name, "SyntaxError", "unexpected error name: {name}");
                    },
                    Err(CaughtError::Error(err)) => {
                        assert!(
                            err.to_string().contains("SyntaxError"),
                            "unexpected: {err:?}"
                        );
                    },
                    Err(CaughtError::Exception(_)) => {},
                    Ok(_) => panic!("expected error"),
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_callable_context_objects() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let function: Value = ctx.eval("(function () {})").unwrap();
                let proxy: Value = ctx.eval("new Proxy(function () {}, {})").unwrap();

                for sandbox in [function, proxy] {
                    let result = run_in_new_context_impl(
                        ctx.clone(),
                        "1".into_js(&ctx).unwrap(),
                        Some(sandbox),
                        None,
                    )
                    .catch(&ctx);

                    assert!(result.is_err(), "expected callable sandbox to be rejected");
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_filename_with_embedded_nul_bytes() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let filename = ctx.eval::<String, _>("'left\\u0000right'").unwrap();
                let options = Object::new(ctx.clone()).unwrap();
                options.set("filename", filename).unwrap();

                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "1".into_js(&ctx).unwrap(),
                    None,
                    Some(options.into_value()),
                )
                .catch(&ctx);

                assert!(result.is_err(), "expected filename with NUL to be rejected");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_string_shorthand_filename_with_embedded_nul_bytes() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let filename = ctx.eval::<String, _>("'left\\u0000right'").unwrap();
                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "1".into_js(&ctx).unwrap(),
                    None,
                    Some(filename.into_js(&ctx).unwrap()),
                )
                .catch(&ctx);

                assert!(
                    result.is_err(),
                    "expected string shorthand filename with NUL to be rejected"
                );
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_null_sandbox_when_global_type_error_is_poisoned() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ctx.eval::<(), _>("globalThis.TypeError = () => { throw new Error('poisoned'); }")
                    .unwrap();

                let result = run_in_new_context_impl(
                    ctx.clone(),
                    "1".into_js(&ctx).unwrap(),
                    Some(Value::new_null(ctx.clone())),
                    None,
                )
                .catch(&ctx);

                assert!(result.is_err());
            })
        })
        .await;
    }
}
