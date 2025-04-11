#!/usr/bin/env node
import path from 'path';
import { fileURLToPath } from 'url';
import { spawnSync } from 'child_process';
import os from 'os';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

let platform = os.platform();
platform = platform === 'win32' ? 'windows' : platform;
const arch = os.arch();

if (arch === 'ia32') {
  throw new Error('32-bit architecture is not supported.');
}

const currentLLRT = `llrt-${platform}-${arch}`;

(async function () {
  try {
    const args = process.argv.slice(2);
    console.log(`lexe: calling ${currentLLRT}, args: ${args}`);
    const llrtPath = path.resolve(__dirname, currentLLRT, 'llrt');
    spawnSync(llrtPath, args, { stdio: 'inherit' });
    process.exit(0);
  } catch (e) {
    console.error(e);
    process.exit(1);
  }
})()