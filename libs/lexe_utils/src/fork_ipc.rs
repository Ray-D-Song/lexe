// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Length-prefixed clone_wire frames for child_process IPC (same process, fork + fd).

use std::io::{Read, Write};

use crate::clone_wire::CloneWirePayload;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const FRAME_MESSAGE: u8 = 3;
pub const FRAME_EXIT: u8 = 5;

pub fn write_frame_sync(
    writer: &mut impl Write,
    tag: u8,
    payload: &CloneWirePayload,
) -> std::io::Result<()> {
    writer.write_all(&[tag])?;
    write_u32(writer, payload.body.len() as u32)?;
    writer.write_all(&payload.body)?;
    write_u32(writer, payload.transfers.len() as u32)?;
    for t in &payload.transfers {
        write_u32(writer, t.len() as u32)?;
        writer.write_all(t)?;
    }
    writer.flush()
}

pub async fn write_frame_async(
    writer: &mut (impl AsyncWriteExt + Unpin),
    tag: u8,
    payload: &CloneWirePayload,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    write_frame_sync(&mut buf, tag, payload)?;
    writer.write_all(&buf).await
}

pub fn read_frame_sync(reader: &mut impl Read) -> std::io::Result<(u8, CloneWirePayload)> {
    let mut tag = [0u8; 1];
    reader.read_exact(&mut tag)?;
    let body_len = read_u32_sync(reader)? as usize;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body)?;
    let transfer_count = read_u32_sync(reader)? as usize;
    let mut transfers = Vec::with_capacity(transfer_count);
    for _ in 0..transfer_count {
        let len = read_u32_sync(reader)? as usize;
        let mut bytes = vec![0u8; len];
        reader.read_exact(&mut bytes)?;
        transfers.push(bytes);
    }
    Ok((tag[0], CloneWirePayload { body, transfers }))
}

pub async fn read_frame_async(
    reader: &mut (impl AsyncReadExt + Unpin),
) -> std::io::Result<(u8, CloneWirePayload)> {
    let mut tag = [0u8; 1];
    reader.read_exact(&mut tag).await?;
    let body_len = read_u32_async(reader).await? as usize;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).await?;
    let transfer_count = read_u32_async(reader).await? as usize;
    let mut transfers = Vec::with_capacity(transfer_count);
    for _ in 0..transfer_count {
        let len = read_u32_async(reader).await? as usize;
        let mut bytes = vec![0u8; len];
        reader.read_exact(&mut bytes).await?;
        transfers.push(bytes);
    }
    Ok((tag[0], CloneWirePayload { body, transfers }))
}

fn write_u32(writer: &mut impl Write, v: u32) -> std::io::Result<()> {
    writer.write_all(&v.to_be_bytes())
}

fn read_u32_sync(reader: &mut impl Read) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

async fn read_u32_async(reader: &mut (impl AsyncReadExt + Unpin)) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}
