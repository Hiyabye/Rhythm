import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { parseOptions, validateWorker } from '../scripts/validate-worker.mjs';

// These child processes implement only the protocol. They never load model weights.
const mockWorker = `
import readline from 'node:readline';
const config = JSON.parse(process.argv[2]);
const loaded = config.loaded ?? config.backend;
const status = { engine: 'llama.cpp', build_id: 'mock-test-only', build_backend: config.backend, runtime_available: true, analysis_json_v1: true, loaded_backend: loaded, model: '/mock.gguf' };
const output = { short_summary: '장학금 신청 안내입니다.', summary: '제출 서류와 신청 기간을 확인하세요.', category: '기타', uncertainty: 'unknown', facts: [] };
function reply(request, result) { process.stdout.write(JSON.stringify({ version: 1, id: request.id, ok: true, result }) + '\\n'); }
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const request = JSON.parse(line);
  if (request.method === 'status') {
    if (config.noise) return process.stdout.write('native diagnostic on stdout\\n');
    if (config.oversized) return process.stdout.write('x'.repeat(1024 * 1024 + 1));
    return reply(request, status);
  }
  if (request.method === 'load') return reply(request, status);
  if (request.method === 'infer') {
    if (request.params.max_tokens === 4096) {
      if (config.partialCancellation) process.stdout.write('{"version":1');
      return setTimeout(() => reply(request, { text: 'too late' }), 30_000);
    }
    const structured = request.params.output_mode === 'analysis_json_v1';
    return reply(request, { loaded_backend: loaded, gpu_layers: config.layers ?? (loaded === 'cpu' ? 0 : 20), gpu_device: config.device ?? (loaded === 'vulkan' ? 'NVIDIA mock device' : loaded === 'metal' ? 'Apple mock GPU' : null), text: structured ? (config.badStructure ? '{}' : JSON.stringify(output)) : '로컬 분석 연결을 확인했습니다.', model_reused: true, request_load_ms: 0, generation_ms: 1 });
  }
  if (request.method === 'shutdown') {
    reply(request, { shutdown: true });
    if (config.trailing) process.stdout.write('unframed diagnostic');
    return process.stdout.end(() => process.exit(0));
  }
});
`;

async function runMock(config, policy = {}) {
  const tempBase = realpathSync(tmpdir());
  const root = mkdtempSync(join(tempBase, 'rhythm-worker-validation-'));
  const modelPath = join(root, 'fixture.gguf');
  const reportPath = join(root, 'report.json');
  const fixturePath = join(root, 'worker.mjs');
  writeFileSync(modelPath, 'GGUF mock fixture; no weights');
  writeFileSync(fixturePath, mockWorker);
  const owned = [];
  try {
    const report = await validateWorker({ backend: config.backend, workerPath: process.execPath, modelPath, reportPath, ...policy }, {
      spawnWorker: (_executable, _args, options) => {
        assert.equal(options.windowsHide, true);
        assert.equal(options.shell, undefined);
        const child = spawn(process.execPath, [fixturePath, JSON.stringify(config)], options);
        owned.push(child);
        return child;
      },
    });
    assert.deepEqual(JSON.parse(readFileSync(reportPath, 'utf8')), report);
    assert.match(report.worker_sha256, /^[0-9a-f]{64}$/);
    assert.match(report.model_sha256, /^[0-9a-f]{64}$/);
    for (const child of owned) assert.ok(child.exitCode !== null || child.signalCode !== null, 'owned child was reaped');
    return report;
  } finally {
    for (const child of owned) if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    assert.equal(dirname(realpathSync(root)), tempBase);
    assert.ok(basename(root).startsWith('rhythm-worker-validation-'));
    rmSync(root, { recursive: true, force: true });
  }
}

test('backend is explicit, fallback is Metal-only, and duplicate/path options are rejected', () => {
  const paths = ['--worker', process.execPath, '--model', join(tmpdir(), 'model.gguf'), '--report', join(tmpdir(), 'report.json')];
  assert.throws(() => parseOptions(paths), /--backend must explicitly/);
  assert.throws(() => parseOptions(['--backend', 'cuda', ...paths]), /--backend must explicitly/);
  assert.throws(() => parseOptions(['--backend', 'cpu', ...paths, '--allow-cpu-fallback']), /only for --backend metal/);
  assert.throws(() => parseOptions(['--backend', 'vulkan', ...paths, '--backend', 'cpu']), /Duplicate/);
  assert.throws(() => parseOptions(['--backend', 'cpu', ...paths.slice(0, -2), '--report', 'relative.json']), /absolute/);
  assert.equal(parseOptions(['--backend', 'metal', ...paths, '--allow-cpu-fallback']).allowCpuFallback, true);
});

test('CPU completes lifecycle and cancellation; semantic wrong answers do not fail integration', async () => {
  const report = await runMock({ backend: 'cpu', partialCancellation: true });
  assert.equal(report.status, 'passed');
  assert.deepEqual(report.observed_backends, ['cpu']);
  assert.equal(report.quality.structure_pass, true);
  assert.equal(report.quality.category_pass, false);
  assert.equal(report.quality.deadline_pass, false);
  assert.equal(report.sessions.length, 2);
  assert.deepEqual(report.steps.map(({ name }) => name), ['status', 'load', 'normal_inference', 'structured_inference', 'kill_inflight_and_reap', 'restart_load', 'recovered_inference', 'shutdown']);
  assert.equal(report.steps[4].result.request_inflight_at_termination, true);
});

test('CPU cannot report GPU offload and Vulkan requires NVIDIA with positive offload', async () => {
  assert.match((await runMock({ backend: 'cpu', layers: 1 })).error, /CPU inference reported GPU/);
  assert.match((await runMock({ backend: 'vulkan', layers: 0 })).error, /No positive GPU layer/);
  assert.match((await runMock({ backend: 'vulkan', device: 'Intel mock device' })).error, /NVIDIA/);
  const report = await runMock({ backend: 'vulkan' });
  assert.equal(report.status, 'passed');
  assert.deepEqual(report.observed_backends, ['vulkan']);
});

test('Metal CPU fallback must be explicit and is never labeled Metal verification', async () => {
  const rejected = await runMock({ backend: 'metal', loaded: 'cpu' });
  assert.equal(rejected.status, 'failed');
  assert.match(rejected.error, /CPU fallback requires --allow-cpu-fallback/);
  const fallback = await runMock({ backend: 'metal', loaded: 'cpu' }, { allowCpuFallback: true });
  assert.equal(fallback.status, 'passed_with_cpu_fallback');
  assert.equal(fallback.requested_backend_observed, false);
  assert.equal(fallback.cpu_fallback_observed, true);
  assert.deepEqual(fallback.observed_backends, ['cpu']);
  const metal = await runMock({ backend: 'metal' });
  assert.equal(metal.status, 'passed');
  assert.equal(metal.requested_backend_observed, true);
  assert.deepEqual(metal.observed_backends, ['metal']);
});

test('invalid structure fails integration even when generation and lifecycle work', async () => {
  const report = await runMock({ backend: 'cpu', badStructure: true });
  assert.equal(report.status, 'structure_failed');
  assert.equal(report.quality.structure_pass, false);
});

test('stdout diagnostics, oversized frames and trailing partial frames fail closed', async () => {
  const noise = await runMock({ backend: 'cpu', noise: true });
  assert.equal(noise.status, 'failed');
  assert.match(noise.error, /Invalid worker response/);
  const oversized = await runMock({ backend: 'cpu', oversized: true });
  assert.equal(oversized.status, 'failed');
  assert.match(oversized.error, /1 MiB protocol frame limit/);
  const trailing = await runMock({ backend: 'cpu', trailing: true });
  assert.equal(trailing.status, 'failed');
  assert.match(trailing.error, /unterminated stdout frame/);
});
