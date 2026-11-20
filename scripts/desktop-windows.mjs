import { spawnSync } from 'node:child_process';
import { mkdirSync, copyFileSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { win32 } from 'node:path';
import { checksumLine, remapFlags } from './release-common.mjs';
import { updaterSigningArgs } from './updater-signing.mjs';

// Windows-only paths, so the orchestration tests behave the same on any host.
const { join, resolve } = win32;

const realFiles = { readdir: readdirSync, read: readFileSync, write: writeFileSync, remove: rmSync };

const PRODUCTION_IDENTIFIER = 'io.github.hiyabye.rhythm';
const PRODUCTION_PRODUCT_NAME = 'Rhythm';

// Extract effective top-level metadata from tauri.conf.json plus any --config
// / -c overrides so validation installers can be guarded before staging and
// published under their own productName.
export function resolveBuildIdentity({ cwd, extra, files }) {
  const base = JSON.parse(files.read(join(cwd, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  const effective = {
    identifier: base.identifier ?? PRODUCTION_IDENTIFIER,
    productName: base.productName ?? PRODUCTION_PRODUCT_NAME,
    version: base.version,
  };
  for (let i = 0; i < extra.length; i++) {
    const arg = extra[i];
    let raw = null;
    if (arg === '--config' || arg === '-c') raw = extra[++i];
    else if (arg.startsWith('--config=')) raw = arg.slice('--config='.length);
    else if (arg.startsWith('-c=')) raw = arg.slice('-c='.length);
    if (!raw) continue;
    let parsed = null;
    if (raw.trimStart().startsWith('{')) {
      parsed = JSON.parse(raw);
    } else {
      try {
        const text = files.read(resolve(cwd, raw), 'utf8');
        parsed = JSON.parse(typeof text === 'string' ? text : text.toString('utf8'));
      } catch {
        // Non-JSON or unreadable config paths are left for Tauri to report.
      }
    }
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      if (typeof parsed.identifier === 'string') effective.identifier = parsed.identifier;
      if (typeof parsed.productName === 'string') effective.productName = parsed.productName;
      if (typeof parsed.version === 'string') effective.version = parsed.version;
    }
  }
  return effective;
}

// Windows 11 x64 build: Microsoft VC runtime staging, the pinned llama.cpp
// CPU build, and the per-user NSIS installer. Every external
// effect is injectable so the orchestration is testable on any host.
export function desktop({ platform = process.platform, arch = process.arch, env = process.env,
  args = process.argv.slice(2), cwd = process.cwd(), spawn = spawnSync,
  copy = copyFileSync, mkdir = mkdirSync, files = realFiles, home = homedir() } = {}) {
  if (platform !== 'win32' || arch !== 'x64') throw new Error('Rhythm Windows builds require Windows 11 x64.');
  const flags = [env.RUSTFLAGS, env.CARGO_ENCODED_RUSTFLAGS, env.CARGO_BUILD_RUSTFLAGS,
    env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS].filter(Boolean).join(' ');
  if (/target-cpu\s*=\s*native/.test(flags)) throw new Error('Portable Windows releases cannot use target-cpu=native.');
  const [mode = 'dev', ...extra] = args;
  if (!['dev', 'build'].includes(mode)) throw new Error('Expected dev or build.');
  if (extra.some((arg) => arg === '--target' || arg.startsWith('--target='))) throw new Error('Cross compilation is not supported. Use the x64 MSVC host.');
  const run = (command, argv, capture = false, commandEnv = env) => {
    const result = spawn(command, argv, { cwd, env: commandEnv, windowsHide: true, ...(capture ? { encoding: 'utf8' } : { stdio: 'inherit' }) });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(command + ' failed (' + (result.status ?? 'signal') + ').');
    return result;
  };
  const target = run('rustc', ['-vV'], true).stdout.match(/^host: (.+)$/m)?.[1]?.trim();
  if (target !== 'x86_64-pc-windows-msvc') throw new Error('Install the x86_64-pc-windows-msvc Rust toolchain.');
  const release = mode === 'build' && !extra.includes('--debug');
  const bundles = mode === 'build' && !extra.includes('--no-bundle');
  const distribute = release && !extra.some((arg) => arg === '--no-bundle' || arg.startsWith('--bundles') || arg === '-b');
  const identity = resolveBuildIdentity({ cwd, extra, files });
  if (bundles && identity.identifier !== PRODUCTION_IDENTIFIER && identity.productName === PRODUCTION_PRODUCT_NAME) {
    throw new Error('Non-production Windows installers must set a distinct productName (not "Rhythm") so they cannot overwrite a production installation.');
  }
  // Checked before staging so a half-configured key fails fast.
  const signing = distribute ? updaterSigningArgs(env) : null;
  const buildEnv = release ? releaseEnv(env, cwd, home) : env;
  const targetDir = resolve(cwd, env.CARGO_TARGET_DIR || 'target');
  // Use the explicitly selected host (windows-env.ps1 exports its own host),
  // retaining Windows PowerShell compatibility for other build environments.
  // Stage before compiling so missing tools or denied scripts fail fast.
  const powershell = env.RHYTHM_POWERSHELL || 'powershell.exe';
  try {
    run(powershell, ['-NoProfile', '-File', join(cwd, 'scripts', 'stage-vc-runtime.ps1')]);
  } catch (cause) {
    throw new Error(`Microsoft VC runtime staging failed using ${powershell}. Set RHYTHM_POWERSHELL to the intended PowerShell executable if needed. The selected host's execution policy is unchanged.`, { cause });
  }
  // The llama.cpp DLLs need the VC runtime staged above beside the build.
  run(process.execPath, [join(cwd, 'scripts', 'stage-llama.mjs')]);
  // Earlier installers (other versions, validation builds) must not sit next to the new one.
  if (distribute) files.remove(join(targetDir, 'release', 'bundle'), { recursive: true, force: true });
  // Direct JS invocation preserves argument boundaries without npm.cmd/shell quoting.
  run(process.execPath, [join(cwd, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), mode, ...extra, ...(signing ?? [])], false, buildEnv);
  if (distribute) publishInstaller({ identity, targetDir, files, copy, mkdir, signed: Boolean(signing) });
}

// Release binaries embed source paths in panic locations; map the builder's
// home, Cargo home and checkout to neutral prefixes (the last match wins).
function releaseEnv(env, cwd, home) {
  return remapFlags(env, [
    [home, 'C:\\home'],
    [env.CARGO_HOME ?? join(home, '.cargo'), 'C:\\cargo'],
    [resolve(cwd), 'C:\\rhythm'],
  ]);
}

// The distribution folder holds exactly the installer and its checksum, plus
// the installer's updater signature when the build was signed.
function publishInstaller({ identity, targetDir, files, copy, mkdir, signed }) {
  const name = `${identity.productName}_${identity.version}_x64-setup.exe`;
  const bundle = join(targetDir, 'release', 'bundle', 'nsis');
  const installers = files.readdir(bundle).filter((file) => file.endsWith('.exe'));
  if (installers.length !== 1 || installers[0] !== name) {
    throw new Error(`Expected exactly ${name} in ${bundle}, found: ${installers.join(', ') || 'none'}`);
  }
  const output = join(targetDir, 'release', 'distribution');
  files.remove(output, { recursive: true, force: true });
  mkdir(output, { recursive: true });
  copy(join(bundle, name), join(output, name));
  const { digest, text } = checksumLine(name, files.read(join(output, name)));
  files.write(join(output, `${name}.sha256`), text);
  if (signed) copy(join(bundle, `${name}.sig`), join(output, `${name}.sig`));
  console.log(`Distribution files in ${output}:\n  ${name}\n  ${name}.sha256 (${digest})${signed ? `\n  ${name}.sig` : ''}`);
}
