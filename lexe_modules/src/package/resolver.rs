// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Mutex,
};

use lexe_utils::io::{is_supported_ext, JS_EXTENSIONS};
use once_cell::sync::Lazy;
use rquickjs::{
    loader::{ImportAttributes, Resolver},
    Ctx, Error, Exception, Function, Result,
};
use simd_json::{derived::ValueObjectAccessAsScalar, BorrowedValue, StaticNode};
use tracing::trace;

use crate::module::extensions::{resolve_extension_candidates, static_extension_candidates};
use crate::modules::path;
use crate::{CJS_IMPORT_PREFIX, CJS_LOADER_PREFIX, LEXE_PLATFORM};

fn extension_candidates(ctx: &Ctx<'_>, is_esm: bool) -> Result<Vec<String>> {
    if is_esm {
        Ok(static_extension_candidates())
    } else {
        resolve_extension_candidates(ctx)
    }
}

fn rc_string_to_cow<'a>(rc: Rc<String>) -> Cow<'a, str> {
    match Rc::try_unwrap(rc) {
        Ok(string) => Cow::Owned(string),
        Err(rc) => Cow::Owned((*rc).clone()),
    }
}

#[derive(Clone, Debug)]
struct NodePathList(pub NodePathListValues);

type NodePathListValues = Rc<RefCell<Vec<Rc<str>>>>;

unsafe impl Send for NodePathList {}

impl NodePathList {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }
}

//None entry means that there are no parent modules
type NodeModulePaths = HashMap<Box<str>, Option<NodePathList>>;

static NODE_MODULES_PATHS_CACHE: Lazy<Mutex<NodeModulePaths>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn home_dir() -> Option<PathBuf> {
    #[cfg(not(target_family = "wasm"))]
    {
        home::home_dir()
    }
    #[cfg(target_family = "wasm")]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

static HOME_NODE_MODULES: Lazy<Vec<Box<str>>> = Lazy::new(|| {
    // Add global folders
    let mut paths = Vec::with_capacity(2);
    if let Some(home) = home_dir() {
        let home_node_modules = home.join(".node_modules");
        let home_node_libraries = home.join(".node_libraries");
        if home_node_modules.is_dir() {
            paths.push(Box::from(home_node_modules.to_string_lossy()));
        }
        if home_node_libraries.is_dir() {
            paths.push(Box::from(home_node_libraries.to_string_lossy()));
        }
    }
    paths
});

static FILESYSTEM_ROOT: Lazy<Box<str>> = Lazy::new(|| {
    #[cfg(unix)]
    {
        "/".into()
    }
    #[cfg(windows)]
    {
        if let Some(path) = home_dir() {
            if let Some(std::path::Component::Prefix(prefix)) = path.components().next() {
                return prefix
                    .as_os_str()
                    .to_string_lossy()
                    .into_owned()
                    .into_boxed_str();
            }
        }

        "C:".to_string().into_boxed_str()
    }
    #[cfg(not(any(unix, windows)))]
    {
        "/".into()
    }
});

#[derive(Debug, Default)]
pub struct PackageResolver;

#[allow(clippy::manual_strip)]
impl Resolver for PackageResolver {
    fn resolve(
        &mut self,
        ctx: &Ctx,
        base: &str,
        name: &str,
        _attr: Option<ImportAttributes>,
    ) -> Result<String> {
        if name.starts_with(CJS_IMPORT_PREFIX) {
            return Ok(name.to_string());
        }

        let base = base.trim_start_matches(CJS_IMPORT_PREFIX);

        trace!("Try resolve '{}' from '{}'", name, base);

        require_resolve(ctx, name, base, None, true).map(|name| name.into_owned())
    }
}

// [CJS Reference Implementation](https://nodejs.org/api/modules.html#all-together)
// require(X) from module at path Y
#[allow(clippy::type_complexity)]
pub fn require_resolve<'a>(
    ctx: &Ctx<'_>,
    x: &'a str,
    y: &str,
    hooked_fn: Option<Function<'_>>,
    is_esm: bool,
) -> Result<Cow<'a, str>> {
    require_resolve_with_options(ctx, x, y, hooked_fn, is_esm, None)
}

#[allow(clippy::type_complexity)]
pub fn require_resolve_with_options<'a>(
    ctx: &Ctx<'_>,
    x: &'a str,
    y: &str,
    hooked_fn: Option<Function<'_>>,
    is_esm: bool,
    options_paths: Option<Vec<String>>,
) -> Result<Cow<'a, str>> {
    // trim schema
    let x = x.trim_start_matches("file://");

    // Node tolerates a trailing separator on bare specifiers
    // (`require("string_decoder/")`), and the bundled CommonJS helpers emit
    // exactly that. Normalize it away for bare names only: for paths the
    // trailing slash is meaningful (`require("./dir/")` must stay a directory).
    let x = if !x.starts_with('.') && !x.starts_with('/') {
        x.trim_end_matches('/')
    } else {
        x
    };

    // resolve symlink
    let y = if let Ok(path) = Path::new(y).read_link() {
        if path.is_absolute() {
            path.to_string_lossy().to_string()
        } else {
            [y, "/../", path.to_string_lossy().as_ref()].concat()
        }
    } else {
        y.to_string()
    };
    let y = y.as_str();

    trace!("require_resolve(x, y):({}, {})", x, y);

    // 1'. If X is a bytecode cache,
    if let Some(hooked_resolve) = hooked_fn {
        if let Ok(path) = hooked_resolve.call::<_, String>((x, y)) {
            return Ok(path.into());
        }
    }

    //fast path for when we have supported extensions
    let (_, ext_name) = path::name_extname(x);
    let is_supported_ext = is_supported_ext(ext_name);

    let x_is_absolute = path::is_absolute(x);
    let x_starts_with_current_dir = x.starts_with("./");
    let x_starts_with_parent_dir = x.starts_with("..");

    if is_supported_ext && Path::new(x).is_file() {
        return resolved_by_file_exists(x.into());
    }

    let x_normalized = path::normalize(x);
    if !x_starts_with_parent_dir && is_supported_ext && Path::new(&x_normalized).is_file() {
        return resolved_by_file_exists(x_normalized.into());
    }

    // 2. If X begins with '/'
    let y = if path::is_absolute(x) {
        // a. set Y to be the file system root
        &*FILESYSTEM_ROOT
    } else {
        y
    };

    // Normalize path Y to generate dirname(Y)
    let dirname_y = if Path::new(y).is_dir() {
        path::resolve_path([y].iter())?
    } else {
        let dirname_y = path::dirname(y);
        path::resolve_path([&dirname_y].iter())?
    };

    // 3. If X begins with './' or '/' or '../'
    if x_starts_with_current_dir || x_is_absolute || x_starts_with_parent_dir {
        if let Some(paths) = options_paths.as_ref() {
            if x_starts_with_current_dir || x_starts_with_parent_dir {
                let suffix = if x_starts_with_current_dir {
                    &x[2..]
                } else {
                    x
                };
                for base in paths {
                    let y_plus_x = Rc::new([base.as_str(), "/", suffix].concat());
                    if let Some(path) =
                        continue_on_unresolved_option(load_as_file(ctx, y_plus_x.clone(), is_esm))?
                    {
                        trace!("+- Resolved by `LOAD_AS_FILE` (paths): {}", path);
                        return to_abs_path(path);
                    }
                    if let Some(path) =
                        continue_on_unresolved_option(load_as_directory(ctx, y_plus_x, is_esm))?
                    {
                        trace!("+- Resolved by `LOAD_AS_DIRECTORY` (paths): {}", path);
                        return to_abs_path(path);
                    }
                }
            }
        }

        let y_plus_x = if x_is_absolute {
            x.into()
        } else if x_starts_with_current_dir {
            [&dirname_y, "/", &x[2..]].concat()
        } else {
            [&dirname_y, "/", x].concat()
        };

        let y_plus_x = Rc::new(y_plus_x);

        // a. LOAD_AS_FILE(Y + X)
        if let Some(path) =
            continue_on_unresolved_option(load_as_file(ctx, y_plus_x.clone(), is_esm))?
        {
            trace!("+- Resolved by `LOAD_AS_FILE`: {}", path);
            return to_abs_path(path);
        }
        // b. LOAD_AS_DIRECTORY(Y + X)
        if let Some(path) = continue_on_unresolved_option(load_as_directory(ctx, y_plus_x, is_esm))?
        {
            trace!("+- Resolved by `LOAD_AS_DIRECTORY`: {}", path);
            return to_abs_path(path);
        }

        // c. THROW "not found"
        return Err(Error::new_resolving(y.to_owned(), x.to_owned()));
    }

    // 4. If X begins with '#'
    if x.starts_with('#') {
        // a. LOAD_PACKAGE_IMPORTS(X, dirname(Y))
        if let Some(path) =
            continue_on_unresolved_option(load_package_imports(ctx, x, &dirname_y, is_esm))?
        {
            trace!("+- Resolved by `LOAD_PACKAGE_IMPORTS`: {}", path);
            return Ok(path.into());
        }
    }

    // 5. LOAD_PACKAGE_SELF(X, dirname(Y))
    if let Some(path) =
        continue_on_unresolved_option(load_package_self(ctx, x, &dirname_y, is_esm))?
    {
        trace!("+- Resolved by `LOAD_PACKAGE_SELF`: {}", path);
        return to_abs_path(path.into());
    }

    // 6. LOAD_NODE_MODULES(X, dirname(Y))
    if let Some(paths) = options_paths.as_ref() {
        let mut search_paths = Vec::new();
        if x_starts_with_current_dir || x_is_absolute || x_starts_with_parent_dir {
            search_paths.clone_from(paths);
        } else {
            for path in paths {
                if let Ok(mut nm_paths) = node_module_paths(path) {
                    search_paths.append(&mut nm_paths);
                }
            }
        }
        if let Some(path) = load_from_search_paths(ctx, x, &search_paths, is_esm)? {
            trace!("+- Resolved by `LOAD_FROM_SEARCH_PATHS`: {}", path);
            return Ok(path);
        }
    } else if let Some(path) = load_node_modules(ctx, x, dirname_y, is_esm)? {
        trace!("+- Resolved by `LOAD_NODE_MODULES`: {}", path);
        return Ok(path);
    }

    // 6.5. LOAD_AS_FILE(X)
    if let Ok(Some(path)) = load_as_file(ctx, Rc::new(x.to_owned()), is_esm) {
        trace!("+- Resolved by `LOAD_AS_FILE`: {}", path);
        return to_abs_path(path);
    }

    // 7. THROW "not found"
    Err(Error::new_resolving(y.to_string(), x.to_string()))
}

fn resolved_by_file_exists(path: Cow<'_, str>) -> Result<Cow<'_, str>> {
    trace!("+- Resolved by `FILE`: {}", path);
    to_abs_path(path)
}

fn to_abs_path(path: Cow<'_, str>) -> Result<Cow<'_, str>> {
    // Always normalize so `a/b/../c` and `a/c` share one module identity.
    let normalized = if path::is_absolute(&path) {
        path::normalize(path.as_ref())
    } else {
        path::resolve_path_with_separator([path.as_ref()], true)?
    };
    Ok(if cfg!(windows) {
        path::replace_backslash(normalized).into()
    } else {
        normalized.into()
    })
}

// LOAD_AS_FILE(X)
fn load_as_file<'a>(ctx: &Ctx<'_>, x: Rc<String>, is_esm: bool) -> Result<Option<Cow<'a, str>>> {
    trace!("|  load_as_file(x): {}", x);

    // 1. If X is a file, load X as its file extension format. STOP
    if Path::new(x.as_ref()).is_file() {
        trace!("|  load_as_file(1): {}", x);
        return Ok(Some(rc_string_to_cow(x)));
    }

    let mut base_file = String::with_capacity(x.len() + 4);
    base_file.push_str(x.as_ref());
    let base_file_length = base_file.len();

    let mut base_file = Some(base_file);

    let extension_candidates = extension_candidates(ctx, is_esm)?;

    // 2. If X.js is a file,
    for extension in extension_candidates.iter() {
        if let Some(mut current_file) = base_file.take() {
            current_file.truncate(base_file_length);
            current_file.push_str(extension);

            if Path::new(&current_file).is_file() {
                // a. Find the closest package scope SCOPE to X.
                match find_the_closest_package_scope(&x) {
                    // b. If no scope was found
                    None => {
                        // 1. MAYBE_DETECT_AND_LOAD(X.js)
                        trace!("|  load_as_file(2.b.1): {}", current_file);
                        return Ok(Some(current_file.into()));
                    },
                    Some(path) => {
                        let mut package_json_buf = Vec::new();
                        let package_json =
                            parse_package_json(ctx, path.as_ref(), &mut package_json_buf)?;
                        // c. If the SCOPE/package.json contains "type" field,
                        match package_type_field(&package_json, path.as_ref()) {
                            Ok(Some(_type)) if _type == "module" || _type == "commonjs" => {
                                // 1. If the "type" field is "module", load X.js as an ECMAScript module. STOP.
                                // 2. If the "type" field is "commonjs", load X.js as an CommonJS module. STOP.
                                trace!("|  load_as_file(2.c.[1|2]): {}", current_file);
                                return Ok(Some(current_file.into()));
                            },
                            Ok(_) => {},
                            Err(err) => return Err(err.throw(ctx)),
                        }
                    },
                }
                // d. MAYBE_DETECT_AND_LOAD(X.js)
                trace!("|  load_as_file(2.d): {}", current_file);
                return Ok(Some(current_file.into()));
            }
            base_file = Some(current_file);
        }
    }

    // 3. If X.json is a file, load X.json to a JavaScript Object. STOP
    if let Some(mut current_file) = base_file.take() {
        current_file.truncate(base_file_length);
        current_file.push_str(".json");
        if Path::new(&current_file).is_file() {
            trace!("|  load_as_file(3): {}", current_file);
            return Ok(Some(current_file.into()));
        }
    }

    // 4. If X.node is a file, load X.node as binary addon. STOP
    if let Some(mut current_file) = base_file.take() {
        current_file.truncate(base_file_length);
        current_file.push_str(".node");
        if Path::new(&current_file).is_file() {
            trace!("|  load_as_file(4): {}", current_file);
            return Ok(Some(current_file.into()));
        }
    }

    Ok(None)
}

// LOAD_INDEX(X)
fn load_index<'a>(ctx: &Ctx<'_>, x: Rc<String>, is_esm: bool) -> Result<Option<Cow<'a, str>>> {
    trace!("|  load_index(x): {}", x);

    let mut base_file = String::with_capacity(x.len() + "/index".len() + 4);
    base_file.push_str(x.as_ref());
    base_file.push_str("/index");
    let base_file_length = base_file.len();

    let mut base_file = Some(base_file);

    let extension_candidates = extension_candidates(ctx, is_esm)?;

    // 1. If X/index.js is a file
    for extension in extension_candidates.iter() {
        if let Some(mut file) = base_file.take() {
            file.truncate(base_file_length);
            file.push_str(extension);
            if Path::new(&file).is_file() {
                // a. Find the closest package scope SCOPE to X.
                match find_the_closest_package_scope(&x) {
                    // b. If no scope was found, load X/index.js as a CommonJS module. STOP.
                    None => {
                        trace!("|  load_index(1.b): {}", file);
                        return Ok(Some(file.into()));
                    },
                    // c. If the SCOPE/package.json contains "type" field,
                    Some(path) => {
                        let mut package_json_buf = Vec::new();
                        let package_json =
                            parse_package_json(ctx, path.as_ref(), &mut package_json_buf)?;
                        match package_type_field(&package_json, path.as_ref()) {
                            Ok(Some("module")) => {
                                // 1. If the "type" field is "module", load X/index.js as an ECMAScript module. STOP.
                                trace!("|  load_index(1.c.1): {}", file);
                                return Ok(Some(file.into()));
                            },
                            Ok(_) => {
                                // 2. Else, load X/index.js as an CommonJS module. STOP.
                                trace!("|  load_index(1.c.2): {}", file);
                                return Ok(Some(file.into()));
                            },
                            Err(err) => return Err(err.throw(ctx)),
                        }
                    },
                }
            }

            base_file = Some(file);
        }
    }

    // 2. If X/index.json is a file, parse X/index.json to a JavaScript object. STOP
    if let Some(mut file) = base_file.take() {
        file.truncate(base_file_length);
        file.push_str(".json");
        if Path::new(&file).is_file() {
            trace!("|  load_index(2): {}", file);
            return Ok(Some(file.into()));
        }
    }

    // 3. If X/index.node is a file, load X/index.node as binary addon. STOP
    if let Some(mut file) = base_file.take() {
        file.truncate(base_file_length);
        file.push_str(".node");
        if Path::new(&file).is_file() {
            trace!("|  load_index(3): {}", file);
            return Ok(Some(file.into()));
        }
    }

    Ok(None)
}

// LOAD_AS_DIRECTORY(X)
fn load_as_directory<'a>(
    ctx: &Ctx<'_>,
    x: Rc<String>,
    is_esm: bool,
) -> Result<Option<Cow<'a, str>>> {
    trace!("|  load_as_directory(x): {}", x);

    // 1. If X/package.json is a file,
    let file = [&x, "/package.json"].concat();
    if Path::new(&file).is_file() {
        // a. Parse X/package.json, and look for "main" field.
        let mut package_json_buf = Vec::new();
        let package_json = parse_package_json(ctx, &file, &mut package_json_buf)?;
        // b. If "main" is a falsy value, GOTO 2.
        if let Some(main) = get_string_field(&package_json, "main") {
            // c. let M = X + (json main field)
            let m = Rc::new([&x, "/", main].concat());
            // d. LOAD_AS_FILE(M)
            if let Some(path) = continue_on_unresolved_option(load_as_file(ctx, m.clone(), is_esm))?
            {
                trace!("|  load_as_directory(1.d): {}", path);
                return Ok(Some(path));
            }
            // e. LOAD_INDEX(M)
            if let Some(path) = continue_on_unresolved_option(load_index(ctx, m, is_esm))? {
                trace!("|  load_as_directory(1.e): {}", path);
                return Ok(Some(path));
            }
            // f. LOAD_INDEX(X) DEPRECATED

            // g. THROW "not found"
            return Err(Error::new_resolving("", x.to_string()));
        }
    }

    // 2. LOAD_INDEX(X)
    if let Some(path) = continue_on_unresolved_option(load_index(ctx, x, is_esm))? {
        trace!("|  load_as_directory(2): {}", path);
        return Ok(Some(path));
    }

    Ok(None)
}

/// `node_modules` character codes reversed (matches Node's `nmChars`).
const NM_CHARS: &[u8; 12] = b"seludom_edon";
const NM_LEN: usize = NM_CHARS.len();

/// Returns ordered `node_modules` search directories for a given absolute path,
/// matching Node's `Module._nodeModulePaths` behavior.
pub fn node_module_paths(from: &str) -> Result<Vec<String>> {
    let from = if path::is_absolute(from) {
        path::replace_backslash(from.to_string())
    } else {
        path::resolve_path([from].iter())?
    };

    #[cfg(windows)]
    {
        let bytes = from.as_bytes();
        if from.len() >= 2 && bytes[from.len() - 1] == b'\\' && bytes[from.len() - 2] == b':' {
            return Ok(vec![format!("{from}node_modules")]);
        }
    }

    #[cfg(not(windows))]
    if from == "/" {
        return Ok(vec!["/node_modules".to_string()]);
    }

    let mut paths = Vec::new();
    let mut last = from.len();
    let mut p = 0isize;

    for (i, ch) in from.char_indices().rev() {
        let code = ch as u32;
        #[cfg(windows)]
        let is_sep = code == b'\\' as u32 || code == b'/' as u32 || code == b':' as u32;
        #[cfg(not(windows))]
        let is_sep = code == b'/' as u32;

        if is_sep {
            if p != NM_LEN as isize {
                paths.push(format!("{}/node_modules", &from[..last]));
            }
            last = i;
            p = 0;
        } else if p != -1 {
            if NM_CHARS.get(p as usize) == Some(&(code as u8)) {
                p += 1;
            } else {
                p = -1;
            }
        }
    }

    #[cfg(not(windows))]
    paths.push("/node_modules".to_string());

    Ok(paths)
}

fn collect_node_modules_paths(start: &str) -> NodePathList {
    let mut cache = NODE_MODULES_PATHS_CACHE.lock().unwrap();
    let start = start.to_string().into_boxed_str();

    if let Some(Some(dirs)) = cache.get(&start) {
        return dirs.clone();
    }

    let path = Path::new(start.as_ref());
    let results = NodePathList::new();
    let mut paths_to_cache = Vec::new();
    let mut current = Some(path);
    let mut i = 0;
    let mut last_found_index = 0;

    while let Some(dir) = current {
        let str_dir = dir.to_string_lossy();
        if let Some(dirs) = cache.get(str_dir.as_ref()) {
            if let Some(dirs) = dirs {
                results.0.borrow_mut().extend(dirs.0.borrow().clone());
            }
            last_found_index = i;
            break;
        }
        if dir.file_name().is_some_and(|name| name != "node_modules") {
            let node_modules = dir.join("node_modules");
            if node_modules.is_dir() {
                last_found_index = i;
                results
                    .0
                    .borrow_mut()
                    .push(node_modules.to_string_lossy().into());
            }
        }
        paths_to_cache.push(str_dir);
        current = dir.parent();
        i += 1;
    }

    for (i, path) in paths_to_cache.iter().enumerate() {
        let path = path.to_string().into_boxed_str();
        if i <= last_found_index {
            cache.insert(path, Some(results.clone()));
        } else {
            cache.insert(path, None);
            break;
        }
    }

    results
}

fn search_node_modules_dir<'a>(
    ctx: &Ctx<'_>,
    dir: &str,
    x: &str,
    is_esm: bool,
) -> Result<Option<Cow<'a, str>>> {
    if let Some(path) = continue_on_unresolved(load_package_exports(ctx, x, dir, is_esm))? {
        trace!("|  load_node_modules(2.a): {}", path);
        return Ok(Some(path));
    }
    let dir_slash_x = Rc::new([dir, "/", x].concat());
    if let Some(path) =
        continue_on_unresolved_option(load_as_file(ctx, dir_slash_x.clone(), is_esm))?
    {
        trace!("|  load_node_modules(2.b): {}", path);
        return Ok(Some(path));
    }
    if let Some(path) = continue_on_unresolved_option(load_as_directory(ctx, dir_slash_x, is_esm))?
    {
        trace!("|  load_node_modules(2.c): {}", path);
        return Ok(Some(path));
    }
    Ok(None)
}

// LOAD_NODE_MODULES(X, START)
fn load_node_modules<'a>(
    ctx: &Ctx<'_>,
    x: &str,
    start: String,
    is_esm: bool,
) -> Result<Option<Cow<'a, str>>> {
    trace!("|  load_node_modules(x, start): ({}, {})", x, start);

    let results = collect_node_modules_paths(&start);

    for dir in results.0.borrow().iter() {
        if let Some(path) = search_node_modules_dir(ctx, dir, x, is_esm)? {
            return Ok(Some(path));
        }
    }

    for dir in HOME_NODE_MODULES.iter() {
        if let Some(path) = search_node_modules_dir(ctx, dir, x, is_esm)? {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

fn load_from_search_paths<'a>(
    ctx: &Ctx<'_>,
    x: &str,
    search_paths: &[String],
    is_esm: bool,
) -> Result<Option<Cow<'a, str>>> {
    for dir in search_paths {
        if let Some(path) = search_node_modules_dir(ctx, dir, x, is_esm)? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

// LOAD_PACKAGE_IMPORTS(X, DIR)
fn load_package_imports(ctx: &Ctx<'_>, x: &str, dir: &str, is_esm: bool) -> Result<Option<String>> {
    trace!("|  load_package_imports(x, dir): ({}, {})", x, dir);

    // 1. Find the closest package scope SCOPE to DIR.
    // 2. If no scope was found, return.
    if let Some(path) = find_the_closest_package_scope(dir) {
        let mut package_json_file = Vec::new();
        let package_json = parse_package_json(ctx, path.as_ref(), &mut package_json_file)?;

        // 3. If the SCOPE/package.json "imports" is null or undefined, return.
        // 4. If `--experimental-require-module` is enabled
        //   a. let CONDITIONS = ["node", "require", "module-sync"]
        //   b. Else, let CONDITIONS = ["node", "require"]
        // 5. let MATCH = PACKAGE_IMPORTS_RESOLVE(X, pathToFileURL(SCOPE),
        //   CONDITIONS) <a href="esm.md#resolver-algorithm-specification">defined in the ESM resolver</a>.
        // 6. RESOLVE_ESM_MATCH(MATCH).
        if let Some(module_path) = package_imports_resolve(&package_json, x) {
            trace!("|  load_package_imports(6): {}", module_path);
            let dir = path.as_ref().trim_end_matches("package.json");
            let module_path = to_abs_path(correct_extensions(
                ctx,
                [dir, module_path].concat(),
                is_esm,
            )?)?;
            return Ok(Some(module_path.into()));
        }
    };

    Ok(None)
}

// LOAD_PACKAGE_EXPORTS(X, DIR)
fn load_package_exports<'a>(
    ctx: &Ctx<'_>,
    x: &str,
    dir: &str,
    is_esm: bool,
) -> Result<Cow<'a, str>> {
    trace!("|  load_package_exports(x, dir): ({}, {})", x, dir);
    //1. Try to interpret X as a combination of NAME and SUBPATH where the name
    //   may have a @scope/ prefix and the subpath begins with a slash (`/`).
    let mut n = 1;
    let (mut name, mut scope, mut is_last) = get_name_and_scope(x, n);

    //2. If X does not match this pattern or DIR/NAME/package.json is not a file,
    //   return.
    let mut package_json_path = String::with_capacity(dir.len() + 64);
    package_json_path.push_str(dir);
    package_json_path.push('/');
    let base_path_length = package_json_path.len();

    let mut package_json_exists;

    loop {
        trace!(
            "|  split name and scope(name, scope): ({}, {})",
            name,
            scope
        );
        package_json_path.push_str(scope);
        package_json_path.push_str("/package.json");

        package_json_exists = Path::new(&package_json_path).exists();

        if package_json_exists || is_last {
            break;
        }
        n += 1;
        (name, scope, is_last) = get_name_and_scope(x, n);
        package_json_path.truncate(base_path_length);
    }

    let mut sub_module = None;

    // Candidate DIR/NAME/package.json was missing. For multi-segment names we may
    // still try DIR/X/package.json; for a bare last-segment miss (`name == "."`)
    // this directory simply does not contain the package — keep searching.
    let (scope, name) = if !package_json_exists {
        if name == "." {
            return Err(Error::new_resolving(dir.to_string(), x.to_string()));
        }
        package_json_path.truncate(base_path_length);
        package_json_path.push_str(x);
        package_json_path.push_str("/package.json");
        if !Path::new(&package_json_path).exists() {
            return Err(Error::new_resolving(dir.to_string(), x.to_string()));
        }
        (x, ".")
    } else {
        let base_path = &package_json_path[..base_path_length];

        let trimmed_name = name.trim_start_matches(".");
        let mut path =
            String::with_capacity(base_path.len() + scope.len() + trimmed_name.len() + 4);
        path.push_str(base_path);
        path.push_str(scope);
        if !trimmed_name.is_empty() {
            path.push('/');
        }

        path.push_str(trimmed_name);
        let base_path_length = path.len();

        let mut path = Some(path);

        for ext in JS_EXTENSIONS {
            if let Some(mut current_path) = path.take() {
                current_path.truncate(base_path_length);
                current_path.push_str(ext);

                if Path::new(&current_path).exists() {
                    if *ext == ".mjs" {
                        //we know its an ESM module
                        return Ok(current_path.into());
                    }
                    sub_module = Some(current_path);
                    break;
                }
                path = Some(current_path);
            }
        }
        (scope, name)
    };

    // Final guard: never treat a missing candidate as invalid package config.
    if !Path::new(&package_json_path).is_file() {
        return Err(Error::new_resolving(dir.to_string(), x.to_string()));
    }

    //3. Parse DIR/NAME/package.json, and look for "exports" field.
    //4. If "exports" is null or undefined, return.
    //5. let MATCH = PACKAGE_EXPORTS_RESOLVE(pathToFileURL(DIR/NAME), "." + SUBPATH,
    //   `package.json` "exports", ["node", "require"]) <a href="esm.md#resolver-algorithm-specification">defined in the ESM resolver</a>.
    //6. RESOLVE_ESM_MATCH(MATCH)
    let mut package_json_buf = Vec::new();
    let package_json = parse_package_json(ctx, &package_json_path, &mut package_json_buf)?;

    if let Some(sub_module) = sub_module {
        if package_json.get_str("type") != Some("module") {
            let sub_module = to_abs_path(sub_module.into())?;
            if is_esm {
                return Ok([CJS_LOADER_PREFIX, &sub_module].concat().into());
            }
            return Ok(sub_module);
        }
        return Ok(sub_module.into());
    }

    // Node LOAD_PACKAGE_EXPORTS step 4: without an "exports" map, return no
    // match for package *subpaths* so LOAD_AS_FILE / LOAD_AS_DIRECTORY can
    // resolve nested packages (e.g. next/dist/compiled/semver). Bare package
    // names (`name == "."`) still use package_exports_resolve for main/browser.
    if name != "." && !has_exports_field(&package_json) {
        return Err(Error::new_resolving(dir.to_string(), x.to_string()));
    }

    let (module_path, resolve_path, is_cjs) = package_exports_resolve(&package_json, name, is_esm)?;
    let module_path = resolve_path.unwrap_or_else(|| module_path.to_string());
    let module_path = to_abs_path(correct_extensions(
        ctx,
        [dir, "/", scope, "/", &module_path].concat(),
        is_esm,
    )?)?;

    if is_cjs && is_esm {
        return Ok([CJS_LOADER_PREFIX, &module_path].concat().into());
    }

    Ok(module_path)
}

// LOAD_PACKAGE_SELF(X, DIR)
fn load_package_self(ctx: &Ctx<'_>, x: &str, dir: &str, is_esm: bool) -> Result<Option<String>> {
    trace!("|  load_package_self(x, dir): ({}, {})", x, dir);
    let mut n = 1;
    let (mut name, mut scope, mut is_last) = get_name_and_scope(x, n);

    // 1. Find the closest package scope SCOPE to DIR.
    let mut package_json_file: Vec<u8>;
    let package_json: BorrowedValue;
    let package_json_path: Box<str> = match find_the_closest_package_scope(dir) {
        // 2. If no scope was found, return.
        None => {
            return Ok(None);
        },
        Some(path) => {
            package_json_file = Vec::new();
            package_json = parse_package_json(ctx, path.as_ref(), &mut package_json_file)?;
            // 3. If the SCOPE/package.json "exports" is null or undefined, return.
            loop {
                trace!(
                    "|  split name and scope(name, scope): ({}, {})",
                    name,
                    scope
                );
                // 4. If the SCOPE/package.json "name" is not the first segment of X, return.
                if is_exports_field_exists(&package_json) {
                    if let Some(name) = get_string_field(&package_json, "name") {
                        if name == scope {
                            break path;
                        }
                    }
                }
                if is_last {
                    return Ok(None);
                }
                n += 1;
                (name, scope, is_last) = get_name_and_scope(x, n);
            }
        },
    };
    // 5. let MATCH = PACKAGE_EXPORTS_RESOLVE(pathToFileURL(SCOPE),
    //    "." + X.slice("name".length), `package.json` "exports", ["node", "require"])
    //    <a href="esm.md#resolver-algorithm-specification">defined in the ESM resolver</a>.
    // 6. RESOLVE_ESM_MATCH(MATCH)
    if let Ok((path, resolve_path, _)) = package_exports_resolve(&package_json, name, is_esm) {
        let path = resolve_path.unwrap_or_else(|| path.to_string());
        trace!("|  load_package_self(2.c): {}", path);
        let dir = package_json_path.trim_end_matches("package.json");
        let module_path = correct_extensions(ctx, [dir, &path].concat(), is_esm)?;
        return Ok(Some(module_path.into()));
    }

    Ok(None)
}

fn get_name_and_scope(x: &str, n: usize) -> (&str, &str, bool) {
    if let Some(pos) = (0..n).try_fold(x.len(), |p, _| x[..p].rfind('/')) {
        (&x[pos + 1..], &x[..pos], false)
    } else {
        (".", x, true)
    }
}

// Implementation equivalent to PACKAGE_EXPORTS_RESOLVE including RESOLVE_ESM_MATCH
fn package_exports_resolve<'a>(
    package_json: &'a BorrowedValue<'a>,
    modules_name: &str,
    is_esm: bool,
) -> Result<(&'a str, Option<String>, bool)> {
    let ident = if is_esm { "import" } else { "require" };

    let modules_name = if modules_name != "." {
        &["./", modules_name].concat()
    } else {
        modules_name
    };

    let wildcard = if let Some(pos) = modules_name.rmatch_indices('/').nth(1) {
        let (name, scope, _) = get_name_and_scope(modules_name, pos.0);
        (Some(name), Some([scope, "/*"].concat()))
    } else {
        (None, None)
    };

    if let BorrowedValue::Object(map) = package_json {
        let is_cjs =
            !matches!(map.get("type"), Some(BorrowedValue::String(ref _type)) if _type == "module");

        // Top-level string form: "exports": "./dist/index.js"
        if modules_name == "." {
            if let Some(BorrowedValue::String(path)) = map.get("exports") {
                return Ok((path.as_ref(), None, is_cjs));
            }
        }

        if let Some(BorrowedValue::Object(exports)) = map.get("exports") {
            // exports["."] = "./dist/index.mjs" (string target) — common modern form
            if let Some(BorrowedValue::String(path)) = exports.get(modules_name) {
                return Ok((path.as_ref(), None, is_cjs));
            }
            if let Some(BorrowedValue::Object(name)) = exports.get(modules_name) {
                // Check for exports -> name -> platform(browser or node) -> [import | require]
                if let Some(BorrowedValue::Object(platform)) = name.get(LEXE_PLATFORM.as_str()) {
                    if let Some(BorrowedValue::String(ident)) = platform.get(ident) {
                        return Ok((ident.as_ref(), None, is_cjs));
                    }
                }
                // Check for exports -> name -> [import | require] -> default
                if let Some(BorrowedValue::Object(ident)) = name.get(ident) {
                    if let Some(BorrowedValue::String(default)) = ident.get("default") {
                        return Ok((default.as_ref(), None, is_cjs));
                    }
                }
                // Check for exports -> name -> platform(browser or node)
                if let Some(BorrowedValue::String(platform)) = name.get(LEXE_PLATFORM.as_str()) {
                    return Ok((platform.as_ref(), None, is_cjs));
                }
                // Check for exports -> name -> [import | require]
                if let Some(BorrowedValue::String(ident)) = name.get(ident) {
                    return Ok((ident.as_ref(), None, is_cjs));
                }
                // Check for exports -> name -> default
                if let Some(BorrowedValue::String(default)) = name.get("default") {
                    return Ok((default.as_ref(), None, is_cjs));
                }
            }
            // Check for wildcard pattern
            if let Some(scope) = wildcard.1 {
                // Check for exports -> scope -> platform(browser or node) -> [import | require]
                if let Some(BorrowedValue::Object(name)) = exports.get(scope.as_str()) {
                    if let Some(BorrowedValue::Object(platform)) = name.get(LEXE_PLATFORM.as_str())
                    {
                        if let Some(BorrowedValue::String(ident)) = platform.get(ident) {
                            let resolve_star = replace_star(ident, wildcard.0.unwrap());
                            return Ok((ident.as_ref(), Some(resolve_star), is_cjs));
                        }
                    }
                    // Check for exports -> scope -> [import | require] -> default
                    if let Some(BorrowedValue::Object(ident)) = name.get(ident) {
                        if let Some(BorrowedValue::String(default)) = ident.get("default") {
                            let resolve_star = replace_star(default, wildcard.0.unwrap());
                            return Ok((default.as_ref(), Some(resolve_star), is_cjs));
                        }
                    }
                    // Check for exports -> scope -> platform(browser or node)
                    if let Some(BorrowedValue::String(platform)) = name.get(LEXE_PLATFORM.as_str())
                    {
                        let resolve_star = replace_star(platform, wildcard.0.unwrap());
                        return Ok((platform.as_ref(), Some(resolve_star), is_cjs));
                    }
                    // Check for exports -> scope -> [import | require]
                    if let Some(BorrowedValue::String(ident)) = name.get(ident) {
                        let resolve_star = replace_star(ident, wildcard.0.unwrap());
                        return Ok((ident.as_ref(), Some(resolve_star), is_cjs));
                    }
                    //  Check for exports -> scope -> default
                    if let Some(BorrowedValue::String(default)) = name.get("default") {
                        let resolve_star = replace_star(default, wildcard.0.unwrap());
                        return Ok((default.as_ref(), Some(resolve_star), is_cjs));
                    }
                }
            }
            // Check for exports -> [import | require] -> default
            if let Some(BorrowedValue::Object(ident)) = exports.get(ident) {
                if let Some(BorrowedValue::String(default)) = ident.get("default") {
                    return Ok((default.as_ref(), None, is_cjs));
                }
            }
            // Check for exports -> [import | require]
            if let Some(BorrowedValue::String(ident)) = exports.get(ident) {
                return Ok((ident.as_ref(), None, is_cjs));
            }
            // [CJS only] Check for exports -> default
            if !is_esm {
                if let Some(BorrowedValue::String(default)) = exports.get("default") {
                    return Ok((default.as_ref(), None, is_cjs));
                }
            }
        }
        // Check for platform(browser or node) field
        if let Some(BorrowedValue::String(platform)) = map.get(LEXE_PLATFORM.as_str()) {
            return Ok((platform.as_ref(), None, is_cjs));
        }
        // [ESM only] Check for module field
        if is_esm {
            if let Some(BorrowedValue::String(module)) = map.get("module") {
                return Ok((module.as_ref(), None, is_cjs));
            }
        }
        // Check for main field
        if let Some(BorrowedValue::String(main)) = map.get("main") {
            return Ok((main.as_ref(), None, is_cjs));
        }
    }
    Ok(("./index.js", None, true))
}

fn replace_star(scope: &str, name: &str) -> String {
    scope.replace("*", name)
}

// Implementation equivalent to PACKAGE_IMPORTS_RESOLVE including RESOLVE_ESM_MATCH
fn package_imports_resolve<'a>(
    package_json: &'a BorrowedValue<'a>,
    modules_name: &str,
) -> Option<&'a str> {
    if let BorrowedValue::Object(map) = package_json {
        if let Some(BorrowedValue::Object(imports)) = map.get("imports") {
            if let Some(BorrowedValue::Object(name)) = imports.get(modules_name) {
                // Check for imports -> name -> platform(browser or node)
                if let Some(BorrowedValue::String(platform)) = name.get(LEXE_PLATFORM.as_str()) {
                    return Some(platform.as_ref());
                }
                // Check for imports -> name -> require
                if let Some(BorrowedValue::String(require)) = name.get("require") {
                    return Some(require.as_ref());
                }
                // Check for imports -> name -> module-sync
                if let Some(BorrowedValue::String(module_sync)) = name.get("module-sync") {
                    return Some(module_sync.as_ref());
                }
                // Check for imports -> name -> default
                if let Some(BorrowedValue::String(default)) = name.get("default") {
                    return Some(default.as_ref());
                }
            }
            // Check for imports -> name
            if let Some(BorrowedValue::String(name)) = imports.get(modules_name) {
                return Some(name.as_ref());
            }
        }
    }
    None
}

/// Find the closest `package.json` scope for `start`, matching Node's
/// `LookupPackageScope` / `GetPackageScopeConfig` behavior:
/// - If `start` is a file, search begins in its parent directory.
/// - Walks toward the filesystem root.
/// - Stops at a `node_modules` directory boundary without using
///   `node_modules/package.json` or inheriting a scope above it.
pub(crate) fn find_the_closest_package_scope(start: &str) -> Option<Box<str>> {
    let path = PathBuf::from(start);
    let mut current_dir = if path.is_file() {
        match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => return None,
        }
    } else {
        path
    };

    loop {
        // Align with node_modules.cc: do not use or cross `.../node_modules/package.json`.
        if current_dir.file_name().and_then(|name| name.to_str()) == Some("node_modules") {
            return None;
        }

        let package_json_path = current_dir.join("package.json");
        if package_json_path.is_file() {
            return package_json_path.to_str().map(Box::from);
        }
        if !current_dir.pop() {
            break;
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackageScopeType {
    /// No controlling `package.json` was found.
    NoScope,
    /// A `package.json` exists but has no valid `"type"` field.
    Typeless,
    Module,
    CommonJs,
}

#[derive(Debug)]
pub(crate) enum PackageFormatError {
    Io {
        path: String,
        source: std::io::Error,
    },
    InvalidJson {
        path: String,
    },
}

impl PackageFormatError {
    pub(crate) fn throw(self, ctx: &Ctx<'_>) -> Error {
        let msg = match &self {
            Self::Io { path, source } => {
                format!("Invalid package config {path} ({source}).")
            },
            Self::InvalidJson { path } => {
                format!("Invalid package config {path}.")
            },
        };
        Exception::throw_type(ctx, &msg)
    }
}

/// Read the nearest package scope `"type"`, propagating I/O and JSON errors.
pub(crate) fn read_package_scope_type(
    start: &str,
) -> std::result::Result<PackageScopeType, PackageFormatError> {
    let Some(package_json_path) = find_the_closest_package_scope(start) else {
        return Ok(PackageScopeType::NoScope);
    };
    let path = package_json_path.as_ref().to_string();
    let mut bytes = fs::read(&path).map_err(|source| PackageFormatError::Io {
        path: path.clone(),
        source,
    })?;
    let value = simd_json::to_borrowed_value(&mut bytes)
        .map_err(|_| PackageFormatError::InvalidJson { path: path.clone() })?;
    Ok(match package_type_field(&value, &path)? {
        Some("module") => PackageScopeType::Module,
        Some("commonjs") => PackageScopeType::CommonJs,
        // Missing `type`, or other strings (Node treats unknown strings as none).
        Some(_) | None => PackageScopeType::Typeless,
    })
}

/// Read `"type"` from a package.json value.
///
/// - missing → `Ok(None)`
/// - string → `Ok(Some(...))`
/// - non-string → invalid package config (Node `node_modules.cc`)
fn package_type_field<'a>(
    package_json: &'a BorrowedValue<'a>,
    package_json_path: &str,
) -> std::result::Result<Option<&'a str>, PackageFormatError> {
    let BorrowedValue::Object(map) = package_json else {
        return Ok(None);
    };
    match map.get("type") {
        None => Ok(None),
        Some(BorrowedValue::String(val)) => Ok(Some(val.as_ref())),
        Some(_) => Err(PackageFormatError::InvalidJson {
            path: package_json_path.to_string(),
        }),
    }
}

fn parse_package_json<'a>(
    ctx: &Ctx<'_>,
    path: &str,
    buf: &'a mut Vec<u8>,
) -> Result<BorrowedValue<'a>> {
    *buf = fs::read(path).map_err(|source| {
        PackageFormatError::Io {
            path: path.to_string(),
            source,
        }
        .throw(ctx)
    })?;
    simd_json::to_borrowed_value(buf).map_err(|_| {
        PackageFormatError::InvalidJson {
            path: path.to_string(),
        }
        .throw(ctx)
    })
}

/// Continue searching when resolution simply found no match; propagate all
/// other errors (notably invalid package config).
fn continue_on_unresolved<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(err) if err.is_resolving() => Ok(None),
        Err(err) => Err(err),
    }
}

fn continue_on_unresolved_option<T>(result: Result<Option<T>>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(value),
        Err(err) if err.is_resolving() => Ok(None),
        Err(err) => Err(err),
    }
}

/// Basename extension, matching Node `path.extname` for format detection
/// (dotfiles like `.hidden` are treated as extensionless).
pub(crate) fn file_extname(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(0) | None => "",
        Some(i) => &name[i..],
    }
}

/// Detected load format for a resolved file path.
///
/// For `Cjs`, `allow_esm_detect` is only set for ambiguous (typeless / no-scope)
/// files, matching Node's detect-module behavior. Explicit `type: "commonjs"`
/// and `.cjs` must not fall back to ESM on parse failure.
///
/// `Esm` (`.mjs` / `type: "module"`) is not "syntax detection": when such a
/// file is pulled through the CJS `_compile` path (e.g. `require("./x.mjs")`),
/// a wrapper parse failure must still hand off to the ESM loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetectedFormat {
    Esm,
    Cjs { allow_esm_detect: bool },
}

impl DetectedFormat {
    pub(crate) fn is_cjs(self) -> bool {
        matches!(self, Self::Cjs { .. })
    }

    /// Whether `_compile` may hand off to the ESM loader after CJS parse failure.
    pub(crate) fn allow_esm_loader_fallback(self) -> bool {
        match self {
            Self::Esm => true,
            Self::Cjs { allow_esm_detect } => allow_esm_detect,
        }
    }
}

/// Resolve the load format for an already-resolved file path.
pub(crate) fn detect_file_format(
    path: &str,
) -> std::result::Result<DetectedFormat, PackageFormatError> {
    let ext = file_extname(path);
    if ext == ".mjs" {
        return Ok(DetectedFormat::Esm);
    }
    if ext == ".cjs" {
        return Ok(DetectedFormat::Cjs {
            allow_esm_detect: false,
        });
    }
    if ext == ".js" || ext.is_empty() {
        return Ok(match read_package_scope_type(path)? {
            PackageScopeType::Module => DetectedFormat::Esm,
            PackageScopeType::CommonJs => DetectedFormat::Cjs {
                allow_esm_detect: false,
            },
            PackageScopeType::NoScope | PackageScopeType::Typeless => DetectedFormat::Cjs {
                allow_esm_detect: true,
            },
        });
    }
    // Unknown extensions keep the historical ESM default.
    Ok(DetectedFormat::Esm)
}

fn get_string_field<'a>(package_json: &'a BorrowedValue<'a>, str: &str) -> Option<&'a str> {
    if let BorrowedValue::Object(map) = package_json {
        if let Some(BorrowedValue::String(val)) = map.get(str) {
            return Some(val.as_ref());
        }
    }
    None
}

fn is_exports_field_exists<'a>(package_json: &'a BorrowedValue<'a>) -> bool {
    if let BorrowedValue::Object(map) = package_json {
        if let Some(BorrowedValue::Object(_)) = map.get("exports") {
            return true;
        }
    }
    false
}

fn has_exports_field(package_json: &BorrowedValue<'_>) -> bool {
    match package_json {
        BorrowedValue::Object(map) => match map.get("exports") {
            None => false,
            Some(BorrowedValue::Static(StaticNode::Null)) => false,
            Some(_) => true,
        },
        _ => false,
    }
}

fn correct_extensions<'a>(ctx: &Ctx<'_>, x: String, is_esm: bool) -> Result<Cow<'a, str>> {
    let (x_is_file, x_is_dir) = if let Ok(md) = fs::metadata(&x) {
        (md.is_file(), md.is_dir())
    } else {
        (false, false)
    };

    if x_is_file {
        return Ok(x.into());
    };

    let index = if x_is_dir { "/index" } else { "" };

    let mut base_path = String::with_capacity(x.len() + index.len() + 4); //add capacity for extention
    base_path.push_str(&x);
    base_path.push_str(index);
    let base_path_length = base_path.len();

    let mut path = Some(base_path);
    let extension_candidates = extension_candidates(ctx, is_esm)?;

    for extension in extension_candidates.iter() {
        if let Some(mut current_path) = path.take() {
            current_path.truncate(base_path_length);
            current_path.push_str(extension);
            if Path::new(&current_path).is_file() {
                return Ok(current_path.into());
            }
            path = Some(current_path);
        }
    }
    Ok(x.into())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{
        detect_file_format, file_extname, find_the_closest_package_scope, node_module_paths,
        read_package_scope_type, DetectedFormat, PackageFormatError, PackageScopeType,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lexe-pkg-scope-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_extname_matches_node_style() {
        assert_eq!(file_extname("/a/b/file.js"), ".js");
        assert_eq!(file_extname("/a/b/file.mjs"), ".mjs");
        assert_eq!(file_extname("/a/b/file.cjs"), ".cjs");
        assert_eq!(file_extname("/a/b/file"), "");
        assert_eq!(file_extname("/a/b/.hidden"), "");
        assert_eq!(file_extname("/a/b/file.test.js"), ".js");
    }

    #[test]
    fn find_the_closest_package_scope_stops_at_node_modules() {
        let root = temp_dir("node-modules-boundary");
        fs::write(root.join("package.json"), r#"{"type":"module"}"#).unwrap();

        let pkg = root.join("node_modules").join("dep");
        fs::create_dir_all(pkg.join("dist").join("bin")).unwrap();
        let file = pkg.join("dist").join("bin").join("cli");
        fs::write(&file, "module.exports = 1;\n").unwrap();

        assert!(find_the_closest_package_scope(file.to_str().unwrap()).is_none());
        assert_eq!(
            read_package_scope_type(file.to_str().unwrap()).unwrap(),
            PackageScopeType::NoScope
        );

        fs::write(pkg.join("package.json"), r#"{"name":"dep"}"#).unwrap();
        let scope = find_the_closest_package_scope(file.to_str().unwrap()).unwrap();
        assert!(scope.ends_with("dep/package.json") || scope.ends_with("dep\\package.json"));
        assert_eq!(
            read_package_scope_type(file.to_str().unwrap()).unwrap(),
            PackageScopeType::Typeless
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detect_file_format_respects_extension_and_package_type() {
        let root = temp_dir("load-format");
        fs::write(root.join("package.json"), r#"{"type":"module"}"#).unwrap();
        let module_js = root.join("mod.js");
        let module_ext = root.join("cli");
        fs::write(&module_js, "export default 1;\n").unwrap();
        fs::write(&module_ext, "export default 1;\n").unwrap();
        assert_eq!(
            detect_file_format(module_js.to_str().unwrap()).unwrap(),
            DetectedFormat::Esm
        );
        assert_eq!(
            detect_file_format(module_ext.to_str().unwrap()).unwrap(),
            DetectedFormat::Esm
        );
        assert_eq!(
            detect_file_format(root.join("x.cjs").to_str().unwrap()).unwrap(),
            DetectedFormat::Cjs {
                allow_esm_detect: false
            }
        );
        assert_eq!(
            detect_file_format(root.join("x.mjs").to_str().unwrap()).unwrap(),
            DetectedFormat::Esm
        );

        let cjs_root = temp_dir("load-format-cjs");
        fs::write(cjs_root.join("package.json"), r#"{"type":"commonjs"}"#).unwrap();
        let cjs_js = cjs_root.join("mod.js");
        fs::write(&cjs_js, "export default 1;\n").unwrap();
        assert_eq!(
            detect_file_format(cjs_js.to_str().unwrap()).unwrap(),
            DetectedFormat::Cjs {
                allow_esm_detect: false
            }
        );

        let typeless = temp_dir("load-format-typeless");
        fs::write(typeless.join("package.json"), r#"{}"#).unwrap();
        let typeless_js = typeless.join("mod.js");
        let typeless_cli = typeless.join("cli");
        fs::write(&typeless_js, "module.exports = 1;\n").unwrap();
        fs::write(&typeless_cli, "module.exports = 1;\n").unwrap();
        assert_eq!(
            detect_file_format(typeless_js.to_str().unwrap()).unwrap(),
            DetectedFormat::Cjs {
                allow_esm_detect: true
            }
        );
        assert_eq!(
            detect_file_format(typeless_cli.to_str().unwrap()).unwrap(),
            DetectedFormat::Cjs {
                allow_esm_detect: true
            }
        );

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(cjs_root);
        let _ = fs::remove_dir_all(typeless);
    }

    #[test]
    fn invalid_package_json_is_an_error() {
        let root = temp_dir("invalid-package-json");
        fs::write(root.join("package.json"), "{ not json").unwrap();
        let file = root.join("mod.js");
        fs::write(&file, "module.exports = 1;\n").unwrap();
        match detect_file_format(file.to_str().unwrap()) {
            Err(PackageFormatError::InvalidJson { path }) => {
                assert!(path.ends_with("package.json"));
            },
            other => panic!("expected InvalidJson, got {other:?}"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_string_package_type_is_an_error() {
        let root = temp_dir("non-string-type");
        fs::write(root.join("package.json"), r#"{"type":1}"#).unwrap();
        let file = root.join("mod.js");
        fs::write(&file, "module.exports = 1;\n").unwrap();
        match detect_file_format(file.to_str().unwrap()) {
            Err(PackageFormatError::InvalidJson { path }) => {
                assert!(path.ends_with("package.json"));
            },
            other => panic!("expected InvalidJson, got {other:?}"),
        }

        fs::write(root.join("package.json"), r#"{"type":"whatever"}"#).unwrap();
        assert_eq!(
            detect_file_format(file.to_str().unwrap()).unwrap(),
            DetectedFormat::Cjs {
                allow_esm_detect: true
            }
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn esm_format_allows_loader_fallback() {
        assert!(DetectedFormat::Esm.allow_esm_loader_fallback());
        assert!(DetectedFormat::Cjs {
            allow_esm_detect: true
        }
        .allow_esm_loader_fallback());
        assert!(!DetectedFormat::Cjs {
            allow_esm_detect: false
        }
        .allow_esm_loader_fallback());
    }

    #[test]
    fn to_abs_path_collapses_parent_segments() {
        let collapsed = super::to_abs_path("/a/b/../c/d.js".into()).unwrap();
        assert_eq!(collapsed.as_ref(), "/a/c/d.js");
    }

    #[test]
    fn node_module_paths_returns_ordered_paths() {
        let paths = node_module_paths("/a/b/c/d").unwrap();
        assert_eq!(paths.len(), 5);
        assert!(paths[0].ends_with("/a/b/c/d/node_modules"));
        assert!(paths.last().unwrap().ends_with("/node_modules"));
    }

    #[test]
    fn node_module_paths_root_is_single_entry() {
        assert_eq!(
            node_module_paths("/").unwrap(),
            vec!["/node_modules".to_string()]
        );
    }

    #[test]
    fn node_module_paths_empty_is_dot() {
        let dot = node_module_paths(".").unwrap();
        let empty = node_module_paths("").unwrap_or_else(|_| node_module_paths(".").unwrap());
        assert!(!dot.is_empty());
        assert_eq!(dot.last(), empty.last());
    }

    #[test]
    fn node_module_paths_skips_nested_node_modules_segment() {
        let paths = node_module_paths("/a/node_modules").unwrap();
        assert!(!paths
            .iter()
            .any(|p| p.ends_with("/node_modules/node_modules")));
        assert!(paths.iter().any(|p| p == "/a/node_modules"));
    }
}
