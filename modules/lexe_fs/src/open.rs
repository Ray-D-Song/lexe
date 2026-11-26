// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::path::PathBuf;

use crate::errors::throw_fs_error;
use crate::fd_sync::{throw_invalid_arg_type, throw_invalid_arg_value};
use lexe_utils::result::ResultExt;
use rquickjs::{function::Opt, Ctx, Result, Value};
use tokio::fs::OpenOptions;

use super::file_handle::{crt_flags, crt_flags_for_node_open, FileHandle};

fn parse_open_mode(ctx: &Ctx<'_>, mode: Opt<Value<'_>>) -> Result<u32> {
    let Some(val) = mode.0 else {
        return Ok(0o666);
    };
    if val.is_undefined() {
        return Ok(0o666);
    }
    if let Some(n) = val.as_number() {
        if n.is_finite() && n.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&n) {
            return Ok(n as u32);
        }
        return Err(throw_invalid_arg_value(
            ctx,
            &format!("The argument 'mode' is invalid. Received {n}"),
        ));
    }
    if let Some(js_str) = val.as_string() {
        let s = js_str.to_string()?;
        let trimmed = s.trim();
        let digits = trimmed
            .strip_prefix("0o")
            .or_else(|| trimmed.strip_prefix("0O"))
            .unwrap_or(trimmed);
        return u32::from_str_radix(digits, 8).map_err(|_| {
            throw_invalid_arg_value(
                ctx,
                &format!("The argument 'mode' is invalid. Received '{s}'"),
            )
        });
    }
    Err(throw_invalid_arg_type(
        ctx,
        "The \"mode\" argument must be of type integer",
    ))
}

fn configure_string_flags(
    ctx: &Ctx<'_>,
    flag_str: &str,
    options: &mut std::fs::OpenOptions,
) -> Result<(bool, bool, i32)> {
    let (needs_read, needs_write) = match flag_str {
        "r" | "rs" | "sr" => {
            options.read(true);
            (true, false)
        },
        "r+" | "rs+" | "sr+" => {
            options.read(true).write(true);
            (true, true)
        },
        "w" => {
            options.write(true).create(true).truncate(true);
            (false, true)
        },
        "wx" | "xw" => {
            options.write(true).create_new(true);
            (false, true)
        },
        "w+" => {
            options.read(true).write(true).create(true).truncate(true);
            (true, true)
        },
        "wx+" | "xw+" => {
            options.read(true).write(true).create_new(true);
            (true, true)
        },
        "a" | "as" => {
            options.append(true).create(true);
            (false, true)
        },
        "ax" | "xa" => {
            options.append(true).create_new(true);
            (false, true)
        },
        "a+" | "as+" => {
            options.read(true).append(true).create(true);
            (true, true)
        },
        "ax+" | "xa+" => {
            options.read(true).append(true).create_new(true);
            (true, true)
        },
        _ => {
            return Err(throw_invalid_arg_value(
                ctx,
                &format!("The argument 'flags' is invalid. Received '{flag_str}'"),
            ));
        },
    };
    Ok((needs_read, needs_write, crt_flags_for_node_open(flag_str)))
}

fn configure_numeric_flags(
    ctx: &Ctx<'_>,
    bits: i32,
    options: &mut std::fs::OpenOptions,
) -> Result<(bool, bool, i32)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let acc = bits & libc::O_ACCMODE;
        let has_append = (bits & libc::O_APPEND) != 0;
        let has_creat = (bits & libc::O_CREAT) != 0;
        let has_excl = (bits & libc::O_EXCL) != 0;
        let has_trunc = (bits & libc::O_TRUNC) != 0;

        let (mut crt, needs_read, needs_write_acc) = match acc {
            x if x == libc::O_RDONLY => {
                options.read(true);
                (crt_flags::O_RDONLY | crt_flags::O_BINARY, true, false)
            },
            x if x == libc::O_WRONLY => {
                options.write(true);
                (crt_flags::O_WRONLY | crt_flags::O_BINARY, false, true)
            },
            x if x == libc::O_RDWR => {
                options.read(true).write(true);
                (crt_flags::O_RDWR | crt_flags::O_BINARY, true, true)
            },
            _ => {
                return Err(throw_invalid_arg_value(
                    ctx,
                    &format!("The argument 'flags' is invalid. Received {bits}"),
                ));
            },
        };
        if has_append {
            options.append(true);
            crt |= crt_flags::O_APPEND;
        }
        if has_creat && has_excl {
            options.create_new(true);
        } else if has_creat {
            options.create(true);
        }
        if has_trunc {
            options.truncate(true);
        }

        let handled =
            libc::O_ACCMODE | libc::O_APPEND | libc::O_CREAT | libc::O_EXCL | libc::O_TRUNC;
        let extra = bits & !handled;
        if extra != 0 {
            options.custom_flags(extra);
        }

        let needs_write = needs_write_acc || has_append || has_creat || has_trunc;
        Ok((needs_read, needs_write, crt))
    }
    #[cfg(not(unix))]
    {
        const O_RDONLY: i32 = 0;
        const O_WRONLY: i32 = 1;
        const O_RDWR: i32 = 2;
        const O_APPEND: i32 = 8;
        const O_CREAT: i32 = 256;
        const O_TRUNC: i32 = 512;
        const O_EXCL: i32 = 1024;

        let acc = bits & 3;
        let has_append = (bits & O_APPEND) != 0;
        let has_creat = (bits & O_CREAT) != 0;
        let has_excl = (bits & O_EXCL) != 0;
        let has_trunc = (bits & O_TRUNC) != 0;

        let (mut crt, needs_read, needs_write_acc) = match acc {
            O_RDONLY => {
                options.read(true);
                (crt_flags::O_RDONLY | crt_flags::O_BINARY, true, false)
            },
            O_WRONLY => {
                options.write(true);
                (crt_flags::O_WRONLY | crt_flags::O_BINARY, false, true)
            },
            O_RDWR => {
                options.read(true).write(true);
                (crt_flags::O_RDWR | crt_flags::O_BINARY, true, true)
            },
            _ => {
                return Err(throw_invalid_arg_value(
                    ctx,
                    &format!("The argument 'flags' is invalid. Received {bits}"),
                ));
            },
        };
        if has_append {
            options.append(true);
            crt |= crt_flags::O_APPEND;
        }
        if has_creat && has_excl {
            options.create_new(true);
        } else if has_creat {
            options.create(true);
        }
        if has_trunc {
            options.truncate(true);
        }

        let needs_write = needs_write_acc || has_append || has_creat || has_trunc;
        Ok((needs_read, needs_write, crt))
    }
}

/// Shared flag/mode parsing and permission enforcement for `fs.promises.open` and `fs.openSync`.
pub(crate) fn build_std_open_options(
    ctx: &Ctx<'_>,
    path: &str,
    flags: Opt<Value<'_>>,
    mode: Opt<Value<'_>>,
) -> Result<(std::fs::OpenOptions, i32)> {
    let mut options = std::fs::OpenOptions::new();
    let (needs_read, needs_write, crt) = match flags.0 {
        None => configure_string_flags(ctx, "r", &mut options)?,
        Some(v) if v.is_undefined() => configure_string_flags(ctx, "r", &mut options)?,
        Some(v) if v.is_string() => {
            let s = v.as_string().unwrap().to_string()?;
            configure_string_flags(ctx, &s, &mut options)?
        },
        Some(v) if v.is_number() => {
            let n = v.as_number().unwrap();
            if !n.is_finite() || n.fract() != 0.0 || n < i32::MIN as f64 || n > u32::MAX as f64 {
                return Err(throw_invalid_arg_value(
                    ctx,
                    &format!("The argument 'flags' is invalid. Received {n}"),
                ));
            }
            configure_numeric_flags(ctx, (n as i64) as i32, &mut options)?
        },
        Some(_) => {
            return Err(throw_invalid_arg_type(
                ctx,
                "The \"flags\" argument must be of type string or number",
            ));
        },
    };

    let parsed_mode = parse_open_mode(ctx, mode)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(parsed_mode);
    }
    #[cfg(not(unix))]
    {
        let _ = parsed_mode;
    }

    if needs_read {
        lexe_utils::permissions::ensure_fs_read(ctx, path)?;
    }
    if needs_write {
        lexe_utils::permissions::ensure_fs_write(ctx, path)?;
    }

    Ok((options, crt))
}

pub async fn open<'js>(
    ctx: Ctx<'js>,
    path: String,
    flags: Opt<Value<'js>>,
    mode: Opt<Value<'js>>,
) -> Result<FileHandle> {
    let path_buf = PathBuf::from(&path);
    let path_str = path_buf.to_string_lossy().into_owned();
    let (std_options, crt) = build_std_open_options(&ctx, &path_str, flags, mode)?;
    let options: OpenOptions = std_options.into();

    let file = options
        .open(&path_buf)
        .await
        .map_err(|err| throw_fs_error(&ctx, err, "open", &path_str))?;

    FileHandle::new_with_crt_flags(file, path_buf, crt).or_throw_msg(&ctx, "Cannot open file")
}

#[cfg(test)]
mod tests {
    use lexe_buffer as buffer;
    use lexe_test::{call_test, given_file, test_async_with, ModuleEvaluator};

    use crate::FsPromisesModule;

    #[tokio::test]
    async fn test_file_handle_read() {
        let path = given_file("Hello World")
            .await
            .to_string_lossy()
            .to_string();
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<FsPromisesModule>(ctx.clone(), "fs/promises")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { open } from 'fs/promises';

                        export async function test(path) {
                            let filehandle = null;
                            try {
                                filehandle = await open(path, 'r+');
                                let { buffer } = await filehandle.read();
                                return Array.from(buffer);
                            } finally {
                                await filehandle?.close();
                            }
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<Vec<u8>, _>(&ctx, &module, (path_1,)).await;

                assert!(result.starts_with(b"Hello World"));
            })
        })
        .await;

        tokio::fs::remove_file(path).await.unwrap();
    }
}
