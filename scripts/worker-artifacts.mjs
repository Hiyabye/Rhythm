import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCHEMA = 1;
const PROTOCOL = 1;
const digest = (value) => createHash('sha256').update(value).digest('hex');
const hashFile = (file) => digest(readFileSync(file));
const environmentKey = /^(?:RUSTFLAGS|RUSTDOCFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_RUSTFLAGS|CARGO_BUILD_TARGET|CARGO_BUILD_RUSTC|CARGO_BUILD_RUSTC_WRAPPER|CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER|CARGO_PROFILE_.+|CARGO_TARGET_.+_(?:RUSTFLAGS|LINKER|RUNNER)|RUSTC|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|RUSTUP_TOOLCHAIN|CC(?:_.+)?|CXX(?:_.+)?|CFLAGS(?:_.+)?|CXXFLAGS(?:_.+)?|CPPFLAGS|LDFLAGS|AR(?:_.+)?|CMAKE_.+|LLAMA_.+|GGML_.+|VULKAN_SDK|VULKAN_SDK_VERSION|LIBCLANG_PATH|BINDGEN_EXTRA_CLANG_ARGS(?:_.+)?|SDKROOT|MACOSX_DEPLOYMENT_TARGET|DEVELOPER_DIR|VCToolsInstallDir|VCToolsVersion|VCINSTALLDIR|WindowsSdkDir|WindowsSDKVersion|UCRTVersion|UniversalCRTSdkDir|INCLUDE|LIB|LIBPATH|CL|_CL_|LINK|_LINK_|CUDA_.+|NVCC_.+|METAL_.+)$/i;

function sourceInputs(cwd, env) {
  const inputs = {};
  const add = (name, file, required = false) => {
    if (!existsSync(file)) {
      if (required) throw new Error(`Missing worker build input: ${file}`);
      inputs[name] = null;
      return;
    }
    if (!lstatSync(file).isFile()) throw new Error(`Worker build input must be a regular file: ${file}`);
    inputs[name] = hashFile(file);
  };
  for (const name of ['Cargo.toml', 'Cargo.lock', 'crates/ai-worker/Cargo.toml', 'crates/ai-worker/build.rs']) {
    add(name, join(cwd, name), true);
  }
  const walk = (relative) => {
    for (const entry of readdirSync(join(cwd, relative), { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
      const name = `${relative}/${entry.name}`;
      if (entry.isDirectory()) walk(name);
      else add(name, join(cwd, name), true);
    }
  };
  walk('crates/ai-worker/src');
  for (const name of ['rust-toolchain', 'rust-toolchain.toml', 'src-tauri/Cargo.toml']) add(name, join(cwd, name));
  // Cargo searches ancestors as well as CARGO_HOME. Hash contents only: configs
  // may contain private registry settings that must not enter the manifest.
  for (let directory = resolve(cwd);; directory = dirname(directory)) {
    for (const name of ['config', 'config.toml']) add(`cargo-config:${join(directory, '.cargo', name)}`, join(directory, '.cargo', name));
    if (dirname(directory) === directory) break;
  }
  for (const name of ['config', 'config.toml']) {
    const file = join(env.CARGO_HOME || join(homedir(), '.cargo'), name);
    add(`cargo-home:${name}`, file);
  }
  // A compiler wrapper or CMake toolchain can change without changing its path.
  for (const [name, value] of Object.entries(env).sort(([a], [b]) => a.localeCompare(b, 'en'))) {
    if (/^(?:RUSTC(?:_WORKSPACE)?_WRAPPER|CARGO_BUILD_RUSTC(?:_WORKSPACE)?_WRAPPER|CMAKE_TOOLCHAIN_FILE|CC(?:_.+)?|CXX(?:_.+)?|AR(?:_.+)?)$/i.test(name)
      && value && existsSync(resolve(cwd, value)) && statSync(resolve(cwd, value)).isFile()) {
      add(`tool-file:${name}`, resolve(cwd, value), true);
    }
  }
  // Changing verification/build rules invalidates older artifacts too.
  add('artifact-rules', fileURLToPath(import.meta.url), true);
  return inputs;
}

function nativeTools(env, cwd) {
  const tools = {};
  const pathValue = Object.entries(env).find(([key]) => key.toUpperCase() === 'PATH')?.[1] ?? '';
  for (const name of ['cl', 'clang', 'clang++', 'cmake', 'ninja', 'glslc', 'xcrun']) {
    const executable = name + (process.platform === 'win32' ? '.exe' : '');
    const found = pathValue.split(delimiter).filter(Boolean).map((directory) => resolve(cwd, directory.replace(/^"|"$/g, ''), executable))
      .find((file) => existsSync(file) && statSync(file).isFile());
    tools[name] = found ? { path: found, sha256: hashFile(found) } : null;
  }
  if (env.LIBCLANG_PATH) {
    const file = join(env.LIBCLANG_PATH, process.platform === 'win32' ? 'libclang.dll' : 'libclang.dylib');
    tools.libclang = existsSync(file) ? { path: file, sha256: hashFile(file) } : null;
  }
  return tools;
}

export function workerIdentity({ cwd, env, target, profile, cargoArgs, rustIdentity, cargoIdentity, engine, backend }) {
  if (!['debug', 'release'].includes(profile)) throw new Error(`Unsupported worker profile: ${profile}`);
  // Redirecting Cargo to another target would put the executable somewhere else
  // and can leave an apparently successful old host executable behind.
  if (env.CARGO_BUILD_TARGET && env.CARGO_BUILD_TARGET !== target) throw new Error('Cross compilation is not supported for worker artifacts.');
  const buildEnvironment = Object.fromEntries(Object.keys(env).sort().filter((key) => environmentKey.test(key))
    .map((key) => [key, digest(String(env[key]))]));
  const inputs = { schema: SCHEMA, protocol: PROTOCOL, target, profile, engine, backend,
    cargoArgs, rustIdentity: rustIdentity.trim(), cargoIdentity: cargoIdentity.trim(),
    environment: buildEnvironment, nativeTools: nativeTools(env, cwd), sources: sourceInputs(cwd, env) };
  return { fingerprint: digest(JSON.stringify(inputs)), inputs };
}

export function probeWorker(binary, { fingerprint, engine, backend, cwd, env, spawn = spawnSync }) {
  const input = [
    { version: PROTOCOL, id: 1, method: 'status', params: {} },
    { version: PROTOCOL, id: 2, method: 'shutdown', params: {} },
  ].map((request) => JSON.stringify(request)).join('\n') + '\n';
  const result = spawn(binary, [], { cwd, env, windowsHide: true, encoding: 'utf8', input,
    timeout: 15_000, maxBuffer: 256 * 1024 });
  if (result.error) throw new Error(`Worker verification failed: ${binary}`, { cause: result.error });
  if (result.status !== 0) throw new Error(`Worker verification exited with ${result.status ?? 'signal'}: ${binary}`);
  let frames;
  try { frames = result.stdout.trim().split(/\r?\n/).map((line) => JSON.parse(line)); }
  catch (cause) { throw new Error(`Worker verification returned invalid JSON: ${binary}`, { cause }); }
  if (frames.length !== 2 || frames.some((frame, index) => frame.version !== PROTOCOL || frame.id !== index + 1 || frame.ok !== true)) {
    throw new Error(`Worker verification returned an incompatible protocol: ${binary}`);
  }
  const status = frames[0].result;
  if (!status || status.build_id !== fingerprint || !/^[a-f0-9]{64}$/.test(status.build_id)
    || status.engine !== engine || status.build_backend !== backend
    || status.runtime_available !== true || status.analysis_json_v1 !== true) {
    throw new Error(`Worker verification returned an incompatible build or runtime: ${binary}`);
  }
  return { protocol: PROTOCOL, build_id: status.build_id, engine: status.engine,
    build_backend: status.build_backend, runtime_available: true, analysis_json_v1: true };
}

// Metadata describes a locally built executable, not an executable path to
// trust. All paths and expected capabilities come from the caller's build plan.
export function ensureWorkerArtifact({ cwd, env, target, profile, cargoArgs, rustIdentity,
  engine, backend, binary, staged, run, spawn = spawnSync,
  copy = copyFileSync, mkdir = mkdirSync, log = console.log }) {
  if (!isAbsolute(binary) || !isAbsolute(staged)) throw new Error('Worker artifact paths must be absolute.');
  const cargoIdentity = run('cargo', ['--version'], true, env).stdout;
  const identityOptions = { cwd, env, target, profile, cargoArgs, rustIdentity, cargoIdentity, engine, backend };
  const { fingerprint, inputs } = workerIdentity(identityOptions);
  const manifestPath = `${staged}.artifact.json`;
  const expected = { fingerprint, engine, backend, cwd, env, spawn };
  try {
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
    if (manifest.schema !== SCHEMA || manifest.fingerprint !== fingerprint
      || manifest.sha256 !== hashFile(staged)) throw new Error('Worker artifact inputs or file hash changed.');
    const status = probeWorker(staged, expected);
    // Also reject a concurrent replacement while the process was being checked.
    if (manifest.sha256 !== hashFile(staged)) throw new Error('Worker changed during verification.');
    log(`Reusing verified ${backend} worker (${fingerprint.slice(0, 12)}).`);
    return { reused: true, fingerprint, staged, manifestPath, status };
  } catch (error) {
    log(`Building ${backend} worker: ${error.code === 'ENOENT' ? 'no verified artifact' : error.message}`);
  }
  run('cargo', cargoArgs, false, { ...env, RHYTHM_WORKER_BUILD_ID: fingerprint });
  const sha256 = hashFile(binary);
  const status = probeWorker(binary, expected);
  // Do not bless a build if sources/configuration changed while Cargo ran.
  if (workerIdentity(identityOptions).fingerprint !== fingerprint) throw new Error('Worker build inputs changed during compilation. Retry the build.');
  mkdir(dirname(staged), { recursive: true });
  copy(binary, staged);
  if (sha256 !== hashFile(staged)) throw new Error('Worker executable changed while staging. Retry the build.');
  // Check the staged sidecar directory too: required DLLs may be resolved
  // differently here than next to Cargo's output executable.
  probeWorker(staged, expected);
  if (sha256 !== hashFile(staged)) throw new Error('Worker changed during verification.');
  const temporary = `${manifestPath}.${process.pid}.tmp`;
  try {
    writeFileSync(temporary, JSON.stringify({ schema: SCHEMA, fingerprint, sha256, status, inputs }, null, 2) + '\n', { flag: 'wx' });
    renameSync(temporary, manifestPath);
  } finally {
    rmSync(temporary, { force: true });
  }
  return { reused: false, fingerprint, staged, manifestPath, status };
}
