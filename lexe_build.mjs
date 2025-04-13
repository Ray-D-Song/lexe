#!/usr/bin/env node
import { mkdir, copyFile, unlink, rm, stat } from 'fs/promises';
import { spawnSync } from 'child_process'
import path from 'path'
import extractZip from 'extract-zip'
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

(async function() {
  try {
    const args = process.argv.slice(2);
    const platform = args[0]; // platform is optional
    
    const allPlatforms = [
      'linux-x64',
      'linux-arm64',
      'darwin-x64',
      'darwin-arm64',
      'windows-x64',
    ];
    
    // if platform is provided, only build the specified platform, otherwise build all platforms
    const platformsToBuild = platform ? platform.split(',') : allPlatforms;
    
    // generate file names based on platforms
    const files = platformsToBuild.map(p => `llrt-${p}-no-sdk.zip`);
    
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
    console.log(`build: building for platforms: ${platformsToBuild.join(', ')}`);
    
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