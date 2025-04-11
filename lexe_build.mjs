#!/usr/bin/env node
import { mkdir, copyFile, unlink, rm, stat } from 'fs/promises';
import { spawnSync } from 'child_process'
import path from 'path'
import extractZip from 'extract-zip'
import { fileURLToPath } from 'url';
import esbuild from 'esbuild'

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

(async function() {
  try {
    const args = process.argv.slice(2);
    const isTestMode = args.includes('test');
    
    const files = isTestMode 
      ? ['llrt-darwin-arm64-no-sdk.zip'] 
      : [
          'llrt-linux-x64-no-sdk.zip',
          'llrt-linux-arm64-no-sdk.zip',
          'llrt-darwin-x64-no-sdk.zip',
          'llrt-darwin-arm64-no-sdk.zip',
          'llrt-windows-x64-no-sdk.zip',
          'llrt-windows-arm64-no-sdk.zip',
        ];
    
    const distDir = path.resolve(__dirname, 'dist');
    try {
      const s = await stat(distDir);
      // check if the dist directory exists
      if (s.isDirectory()) {
        // delete the dist directory
        await rm(distDir, {
          recursive: true
        });
      }
    } catch(e) {}
    await mkdir(distDir, {
      recursive: true,
    });
    console.log('build: dist/ created successfully.');
    console.log(`build: running in ${isTestMode ? 'test' : 'normal'} mode`);
    
    // clean llrt zip
    for (const file of files) {
      const llrtPath = path.resolve(__dirname, file);
      try {
        await unlink(llrtPath)
      } catch(e) {}
    }
    spawnSync('make', files, {
      stdio: 'inherit',
      shell: true,
      cwd: __dirname,
    }); 

    // copy files to dist directory
    await copyAndUnzip(files, distDir);


    // copy lexe_bin.mjs to dist directory
    await copyFile(path.resolve(__dirname, 'lexe_bin.mjs'), path.resolve(distDir, 'bin.mjs'));
  } catch (error) {
    console.log(error)
  }
})()

async function copyAndUnzip(filesToCopy, distDir) {
  for (const file of filesToCopy) {
    const llrtPath = path.resolve(__dirname, file);
    const destPath = path.resolve(distDir, file);

    await copyFile(llrtPath, destPath);
    // unzip the file
    const extractDir = path.resolve(distDir, file.replace('.zip', '').replace('-no-sdk', ''));
    await extractZip(destPath, { dir: extractDir });
    await unlink(destPath);
  }
}