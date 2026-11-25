// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
use std::{fs::Metadata, path::PathBuf};

use lexe_context::CtxExtension;
use lexe_path::{ends_with_sep, CURRENT_DIR_STR};
use lexe_utils::fs::DirectoryWalker;
use rquickjs::{
    atom::PredefinedAtom, function::Rest, prelude::Opt, Array, Class, Ctx, Exception, FromJs,
    Function, IntoJs, Object, Result, Value,
};

use crate::errors::{create_fs_error, defer_fs_callback, throw_fs_error};

#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct Dirent {
    #[qjs(skip_trace)]
    metadata: Metadata,
}

impl Dirent {
    #[allow(dead_code)]
    pub(crate) fn new(metadata: Metadata) -> Self {
        Self { metadata }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Dirent {
    pub fn is_file(&self) -> bool {
        self.metadata.is_file()
    }
    pub fn is_directory(&self) -> bool {
        self.metadata.is_dir()
    }

    pub fn is_symbolic_link(&self) -> bool {
        self.metadata.is_symlink()
    }

    #[qjs(rename = "isFIFO")]
    pub fn is_fifo(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_fifo()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_block_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_block_device()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_character_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_char_device()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_socket(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_socket()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

struct ReadDirItem {
    name: String,
    metadata: Option<Metadata>,
}

pub struct ReadDir {
    items: Vec<ReadDirItem>,
    root: String,
}

impl<'js> IntoJs<'js> for ReadDir {
    fn into_js(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        let arr = Array::new(ctx.clone())?;
        for (index, item) in self.items.into_iter().enumerate() {
            if let Some(metadata) = item.metadata {
                let dirent = Dirent { metadata };

                let dirent = Class::instance(ctx.clone(), dirent)?;
                dirent.set(PredefinedAtom::Name, item.name)?;
                dirent.set("parentPath", &self.root)?;
                arr.set(index, dirent)?;
            } else {
                arr.set(index, item.name)?;
            }
        }
        arr.into_js(ctx)
    }
}

pub async fn read_dir(ctx: Ctx<'_>, mut path: String, options: Opt<Object<'_>>) -> Result<ReadDir> {
    let (with_file_types, skip_root_pos, mut directory_walker) =
        process_options_and_create_directory_walker(&mut path, options);

    let mut items = Vec::with_capacity(64);

    loop {
        match directory_walker.walk().await {
            Ok(Some((child, metadata))) => {
                append_directory_and_metadata_to_vec(
                    with_file_types,
                    skip_root_pos,
                    &mut items,
                    child,
                    metadata,
                );
            },
            Ok(None) => break,
            Err(err) => return Err(throw_fs_error(&ctx, err, "scandir", &path)),
        }
    }

    items.sort_by(|a, b| a.name.partial_cmp(&b.name).unwrap());

    Ok(ReadDir { items, root: path })
}

/// Callback-style `fs.readdir(path[, options], callback)`.
pub fn readdir_callback<'js>(ctx: Ctx<'js>, path: String, args: Rest<Value<'js>>) -> Result<()> {
    let mut args = args.0;
    let (options, callback) = match args.len() {
        0 => {
            return Err(Exception::throw_type(
                &ctx,
                "The \"cb\" argument must be of type function",
            ));
        },
        1 => {
            let only = args.remove(0);
            if only.as_function().is_some() {
                (None, Function::from_js(&ctx, only)?)
            } else {
                return Err(Exception::throw_type(
                    &ctx,
                    "The \"cb\" argument must be of type function",
                ));
            }
        },
        _ => {
            let maybe_opts = args.remove(0);
            if maybe_opts.as_function().is_some() {
                (None, Function::from_js(&ctx, maybe_opts)?)
            } else {
                let opts = if maybe_opts.is_null() || maybe_opts.is_undefined() {
                    None
                } else {
                    Some(Object::from_js(&ctx, maybe_opts)?)
                };
                let callback = Function::from_js(&ctx, args.remove(0)).map_err(|_| {
                    Exception::throw_type(&ctx, "The \"cb\" argument must be of type function")
                })?;
                (opts, callback)
            }
        },
    };

    ctx.clone().spawn_exit_simple(async move {
        // Inline the walk so callback failures can use create_fs_error without
        // going through throw_fs_error (which is for sync Result throws).
        let mut path_mut = path.clone();
        let walk_result = async {
            let (with_file_types, skip_root_pos, mut directory_walker) =
                process_options_and_create_directory_walker(&mut path_mut, Opt(options));
            let mut items = Vec::with_capacity(64);
            loop {
                match directory_walker.walk().await {
                    Ok(Some((child, metadata))) => {
                        append_directory_and_metadata_to_vec(
                            with_file_types,
                            skip_root_pos,
                            &mut items,
                            child,
                            metadata,
                        );
                    },
                    Ok(None) => break,
                    Err(err) => return Err(err),
                }
            }
            items.sort_by(|a, b| a.name.partial_cmp(&b.name).unwrap());
            Ok(ReadDir {
                items,
                root: path_mut,
            })
        }
        .await;

        match walk_result {
            Ok(entries) => {
                let value = entries.into_js(&ctx)?;
                defer_fs_callback(&ctx, callback, None, Some(value))?;
            },
            Err(err) => {
                let exception = create_fs_error(&ctx, err, "scandir", &path)?;
                defer_fs_callback(&ctx, callback, Some(exception), None)?;
            },
        }
        Ok(())
    });

    Ok(())
}

pub fn read_dir_sync(ctx: Ctx<'_>, mut path: String, options: Opt<Object<'_>>) -> Result<ReadDir> {
    let (with_file_types, skip_root_pos, mut directory_walker) =
        process_options_and_create_directory_walker(&mut path, options);

    let mut items = Vec::with_capacity(64);
    loop {
        match directory_walker.walk_sync() {
            Ok(Some((child, metadata))) => {
                append_directory_and_metadata_to_vec(
                    with_file_types,
                    skip_root_pos,
                    &mut items,
                    child,
                    metadata,
                );
            },
            Ok(None) => break,
            Err(err) => return Err(throw_fs_error(&ctx, err, "scandir", &path)),
        }
    }

    items.sort_by(|a, b| a.name.partial_cmp(&b.name).unwrap());

    Ok(ReadDir { items, root: path })
}

type OptionsAndDirectoryWalker = (bool, usize, DirectoryWalker<fn(&str) -> bool>);

fn process_options_and_create_directory_walker(
    path: &mut String,
    options: Opt<Object>,
) -> OptionsAndDirectoryWalker {
    let mut with_file_types = false;
    let mut is_recursive = false;

    if let Some(options) = options.0 {
        with_file_types = options
            .get("withFileTypes")
            .ok()
            .and_then(|file_types: Value| file_types.as_bool())
            .unwrap_or_default();

        is_recursive = options
            .get("recursive")
            .ok()
            .and_then(|recursive: Value| recursive.as_bool())
            .unwrap_or_default();
    };

    if ends_with_sep(path) {
        path.pop();
    }

    let skip_root_pos = {
        match path.as_str() {
            // . | ./
            "." | CURRENT_DIR_STR => path.len(),
            // path
            _ => path.len() + 1,
        }
    };

    let mut directory_walker: DirectoryWalker<fn(&str) -> bool> =
        DirectoryWalker::new(PathBuf::from(&path), |_| true);

    if is_recursive {
        directory_walker.set_recursive(true);
    }
    (with_file_types, skip_root_pos, directory_walker)
}

fn append_directory_and_metadata_to_vec(
    with_file_types: bool,
    skip_root_pos: usize,
    items: &mut Vec<ReadDirItem>,
    child: PathBuf,
    metadata: Metadata,
) {
    let metadata = if with_file_types {
        Some(metadata)
    } else {
        None
    };

    let name = child.into_os_string().to_string_lossy()[skip_root_pos..].to_string();

    items.push(ReadDirItem { name, metadata })
}
