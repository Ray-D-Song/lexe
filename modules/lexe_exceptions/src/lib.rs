// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::{
    option::Undefined,
    primordials::{BasePrimordials, Primordial},
};
use rquickjs::{
    atom::PredefinedAtom,
    class::JsClass,
    function::{Constructor, Opt},
    object::Property,
    prelude::This,
    Class, Coerced, Ctx, Exception, IntoJs, Object, Result, Value,
};

#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct DOMException {
    name: String,
    message: String,
    stack: String,
    code: u8,
}

fn add_constants(obj: &Object<'_>) -> Result<()> {
    const CONSTANTS: [(&str, u8); 25] = [
        ("INDEX_SIZE_ERR", 1),
        ("DOMSTRING_SIZE_ERR", 2),
        ("HIERARCHY_REQUEST_ERR", 3),
        ("WRONG_DOCUMENT_ERR", 4),
        ("INVALID_CHARACTER_ERR", 5),
        ("NO_DATA_ALLOWED_ERR", 6),
        ("NO_MODIFICATION_ALLOWED_ERR", 7),
        ("NOT_FOUND_ERR", 8),
        ("NOT_SUPPORTED_ERR", 9),
        ("INUSE_ATTRIBUTE_ERR", 10),
        ("INVALID_STATE_ERR", 11),
        ("SYNTAX_ERR", 12),
        ("INVALID_MODIFICATION_ERR", 13),
        ("NAMESPACE_ERR", 14),
        ("INVALID_ACCESS_ERR", 15),
        ("VALIDATION_ERR", 16),
        ("TYPE_MISMATCH_ERR", 17),
        ("SECURITY_ERR", 18),
        ("NETWORK_ERR", 19),
        ("ABORT_ERR", 20),
        ("URL_MISMATCH_ERR", 21),
        ("QUOTA_EXCEEDED_ERR", 22),
        ("TIMEOUT_ERR", 23),
        ("INVALID_NODE_TYPE_ERR", 24),
        ("DATA_CLONE_ERR", 25),
    ];

    for (key, value) in CONSTANTS {
        obj.prop(key, Property::from(value).enumerable())?;
    }

    Ok(())
}

impl<'js> JsClass<'js> for DOMException {
    const NAME: &'static str = "DOMException";
    type Mutable = rquickjs::class::Writable;
    fn prototype(ctx: &Ctx<'js>) -> rquickjs::Result<Option<Object<'js>>> {
        use rquickjs::class::impl_::{MethodImpl, MethodImplementor};
        let proto = Object::new(ctx.clone())?;
        let implementor = MethodImpl::<Self>::new();
        implementor.implement(&proto)?;
        add_constants(&proto)?;

        Ok(Some(proto))
    }
    fn constructor(ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        use rquickjs::class::impl_::{ConstructorCreate, ConstructorCreator};
        let implementor = ConstructorCreate::<Self>::new();
        let constructor = implementor
            .create_constructor(ctx)?
            .expect("DOMException must have a constructor");
        add_constants(&constructor)?;

        Ok(Some(constructor))
    }
}

impl<'js> IntoJs<'js> for DOMException {
    fn into_js(self, ctx: &rquickjs::Ctx<'js>) -> Result<Value<'js>> {
        let cls = Class::<Self>::instance(ctx.clone(), self)?;
        rquickjs::IntoJs::into_js(cls, ctx)
    }
}

impl<'js> rquickjs::FromJs<'js> for DOMException
where
    for<'a> rquickjs::class::impl_::CloneWrapper<'a, Self>:
        rquickjs::class::impl_::CloneTrait<Self>,
{
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        use rquickjs::class::impl_::{CloneTrait, CloneWrapper};
        let value = Class::<Self>::from_js(ctx, value)?;
        let borrow = value.try_borrow()?;
        Ok(CloneWrapper(&*borrow).wrap_clone())
    }
}

#[rquickjs::methods]
impl DOMException {
    #[qjs(constructor)]
    pub fn new(
        ctx: Ctx<'_>,
        this: This<Value<'_>>,
        message: Opt<Undefined<Coerced<String>>>,
        name: Opt<Undefined<Coerced<String>>>,
    ) -> Result<Self> {
        if this.0.is_undefined() {
            return Err(Exception::throw_type(
                &ctx,
                "Cannot call the DOMException constructor without 'new'",
            ));
        }

        let message = match message.0 {
            Some(Undefined(Some(message))) => message.0,
            _ => String::new(),
        };

        let name = match name.0 {
            Some(Undefined(Some(message))) => DOMExceptionName::from(message.0),
            _ => DOMExceptionName::Error,
        };

        Self::new_with_name(&ctx, name, message)
    }

    #[qjs(skip)]
    pub fn new_with_name(ctx: &Ctx<'_>, name: DOMExceptionName, message: String) -> Result<Self> {
        let primordials = BasePrimordials::get(ctx)?;

        let new: Object = primordials
            .constructor_error
            .construct((message.clone(),))?;

        Ok(Self {
            name: name.as_str().to_string(),
            code: name.code(),
            message,
            stack: new.get::<_, String>(PredefinedAtom::Stack)?,
        })
    }

    #[qjs(get, enumerable, configurable)]
    fn message(&self) -> &str {
        self.message.as_str()
    }

    #[qjs(get, enumerable, configurable)]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    #[qjs(get, enumerable, configurable)]
    pub fn code(&self) -> u8 {
        self.code
    }

    #[qjs(get)]
    fn stack(&self) -> String {
        self.stack.clone()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &str {
        stringify!(DOMException)
    }
}

macro_rules! create_dom_exception {
    ($name:ident, $($variant:ident),+ $(,)?) => {
        #[derive(Debug)]
        pub enum $name {
            $(
                $variant,
            )+
            Other(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(
                        Self::$variant => stringify!($variant),
                    )+
                    Self::Other(value) => value,
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() {
                    $(
                        stringify!($variant) => Self::$variant,
                    )+
                    _ => Self::Other(value),
                }
            }
        }
    };
}

// https://webidl.spec.whatwg.org/#dfn-error-names-table
create_dom_exception!(
    DOMExceptionName,
    IndexSizeError,
    HierarchyRequestError,
    WrongDocumentError,
    InvalidCharacterError,
    NoModificationAllowedError,
    NotFoundError,
    NotSupportedError,
    InUseAttributeError,
    InvalidStateError,
    SyntaxError,
    InvalidModificationError,
    NamespaceError,
    InvalidAccessError,
    TypeMismatchError,
    SecurityError,
    NetworkError,
    AbortError,
    URLMismatchError,
    QuotaExceededError,
    TimeoutError,
    InvalidNodeTypeError,
    DataCloneError,
    EncodingError,
    NotReadableError,
    UnknownError,
    ConstraintError,
    DataError,
    TransactionInactiveError,
    ReadOnlyError,
    VersionError,
    OperationError,
    NotAllowedError,
    Error,
);

impl DOMExceptionName {
    fn code(&self) -> u8 {
        match self {
            DOMExceptionName::IndexSizeError => 1,
            DOMExceptionName::HierarchyRequestError => 3,
            DOMExceptionName::WrongDocumentError => 4,
            DOMExceptionName::InvalidCharacterError => 5,
            DOMExceptionName::NoModificationAllowedError => 7,
            DOMExceptionName::NotFoundError => 8,
            DOMExceptionName::NotSupportedError => 9,
            DOMExceptionName::InUseAttributeError => 10,
            DOMExceptionName::InvalidStateError => 11,
            DOMExceptionName::SyntaxError => 12,
            DOMExceptionName::InvalidModificationError => 13,
            DOMExceptionName::NamespaceError => 14,
            DOMExceptionName::InvalidAccessError => 15,
            DOMExceptionName::TypeMismatchError => 17,
            DOMExceptionName::SecurityError => 18,
            DOMExceptionName::NetworkError => 19,
            DOMExceptionName::AbortError => 20,
            DOMExceptionName::URLMismatchError => 21,
            DOMExceptionName::QuotaExceededError => 22,
            DOMExceptionName::TimeoutError => 23,
            DOMExceptionName::InvalidNodeTypeError => 24,
            DOMExceptionName::DataCloneError => 25,
            _ => 0,
        }
    }
}

/// QuickJS CallSite only implements a subset of V8's API. Libraries such as
/// `depd` (pulled in by Next's `send`) call `isEval()` / `getEvalOrigin()` and
/// crash if they are missing. Patch the prototype once at startup.
const CALLSITE_POLYFILL_JS: &str = r#"(function () {
  if (typeof Error.captureStackTrace !== "function") {
    return;
  }
  const prev = Error.prepareStackTrace;
  Error.prepareStackTrace = function (_err, stack) {
    return stack;
  };
  try {
    const holder = {};
    Error.captureStackTrace(holder);
    const cs = holder.stack && holder.stack[0];
    if (!cs) {
      return;
    }
    const proto = Object.getPrototypeOf(cs);
    if (!proto || proto.__lexeCallSitePolyfill) {
      return;
    }
    const stubFalse = function () {
      return false;
    };
    const stubNull = function () {
      return null;
    };
    const stubUndef = function () {
      return undefined;
    };
    if (typeof proto.isEval !== "function") proto.isEval = stubFalse;
    if (typeof proto.isNative !== "function") proto.isNative = stubFalse;
    if (typeof proto.isConstructor !== "function") proto.isConstructor = stubFalse;
    if (typeof proto.isToplevel !== "function") proto.isToplevel = stubFalse;
    if (typeof proto.getEvalOrigin !== "function") proto.getEvalOrigin = stubUndef;
    if (typeof proto.getMethodName !== "function") proto.getMethodName = stubNull;
    if (typeof proto.getTypeName !== "function") proto.getTypeName = stubNull;
    if (typeof proto.getThis !== "function") proto.getThis = stubUndef;
    // V8 CallSite#toString — without this, stacks print as "at [object CallSite]".
    if (typeof proto.toString !== "function" || proto.toString === Object.prototype.toString) {
      proto.toString = function () {
        const fn = (typeof this.getFunctionName === "function" && this.getFunctionName()) || "";
        const file = (typeof this.getFileName === "function" && this.getFileName()) || "<anonymous>";
        const line = typeof this.getLineNumber === "function" ? this.getLineNumber() : null;
        const col = typeof this.getColumnNumber === "function" ? this.getColumnNumber() : null;
        const loc =
          line != null ? file + ":" + line + (col != null ? ":" + col : "") : file;
        return fn ? fn + " (" + loc + ")" : loc;
      };
    }
    proto.__lexeCallSitePolyfill = true;
  } finally {
    Error.prepareStackTrace = prev;
  }
})()"#;

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();

    BasePrimordials::init(ctx)?;

    if let Some(constructor) = Class::<DOMException>::create_constructor(ctx)? {
        // the wpt tests expect this particular property descriptor
        globals.prop(
            DOMException::NAME,
            Property::from(constructor).writable().configurable(),
        )?;
    }

    let dom_ex_proto = Class::<DOMException>::prototype(ctx)?.unwrap();
    let error_prototype = &BasePrimordials::get(ctx)?.prototype_error;
    dom_ex_proto.set_prototype(Some(error_prototype))?;

    let _: () = ctx.eval(CALLSITE_POLYFILL_JS)?;

    Ok(())
}
