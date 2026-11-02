// Inspect an explicitly selected profile without writing to it. Run after the
// app has completely exited: node --experimental-sqlite scripts/verify-native-runtime.mjs
// --data-root <absolute-path> [--external-model <absolute-path>]
import { createHash } from 'node:crypto';
import { createReadStream, realpathSync, statSync } from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

function options(args) {
  const result = {};
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i];
    if (!['--data-root', '--external-model'].includes(key) || result[key] !== undefined
      || !args[i + 1] || args[i + 1].startsWith('--') || !path.isAbsolute(args[i + 1])) {
      throw new Error(`Invalid argument: ${key}`);
    }
    result[key] = args[i + 1];
  }
  if (!result['--data-root']) throw new Error('--data-root is required.');
  return result;
}

async function sha256(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

// This limited shape check does not replicate the application's full validator,
// establish semantic quality, or print the source notice or generated text.
function resultShape(raw) {
  try {
    const result = JSON.parse(raw);
    const strings = ['short_summary', 'summary', 'category', 'uncertainty'];
    const valid = result && typeof result === 'object' && !Array.isArray(result)
      && strings.every((key) => typeof result[key] === 'string' && result[key].trim().length > 0)
      && Array.isArray(result.facts)
      && result.facts.every((fact) => fact && typeof fact === 'object' && !Array.isArray(fact)
        && ['kind', 'label', 'value', 'evidence', 'uncertainty'].every((key) => typeof fact[key] === 'string')
        && (fact.date === null || typeof fact.date === 'string'));
    if (!valid) return { basic_shape_valid: false };
    return {
      basic_shape_valid: true,
      category: result.category,
      uncertainty: result.uncertainty,
      short_summary_characters: [...result.short_summary].length,
      summary_characters: [...result.summary].length,
      fact_count: result.facts.length,
      fact_kinds: result.facts.map((fact) => fact.kind),
      facts_with_normalized_dates: result.facts.filter((fact) => typeof fact.date === 'string').length,
    };
  } catch {
    return { basic_shape_valid: false };
  }
}

async function inspect(args) {
  const root = realpathSync(args['--data-root']);
  const database = new DatabaseSync(path.join(root, 'rhythm.sqlite3'), { readOnly: true });
  try {
    const scalar = (sql) => Object.values(database.prepare(sql).get())[0];
    const integrity = database.prepare('PRAGMA integrity_check').all().map((row) => row.integrity_check);
    const foreignKeys = database.prepare('PRAGMA foreign_key_check').all().length;
    const models = database.prepare('SELECT id, ownership, status, bytes, sha256 FROM ai_models ORDER BY id').all();
    const jobs = database.prepare('SELECT id, document_id, revision_id, status, duration_ms, model_sha256, prompt_version, schema_version FROM ai_jobs ORDER BY id').all();
    const results = database.prepare('SELECT id, job_id, document_id, revision_id, model_sha256, prompt_version, schema_version, partial, processed_chars, total_chars, duration_ms, result_json FROM ai_results ORDER BY id').all()
      .map(({ result_json, ...row }) => ({ ...row, inspection: resultShape(result_json) }));
    let external = null;
    if (args['--external-model']) {
      const file = realpathSync(args['--external-model']);
      const selectedStat = statSync(file, { bigint: true });
      if (!selectedStat.isFile()) throw new Error('Explicit external model is not a regular file.');
      if (selectedStat.ino === 0n) throw new Error('Cannot establish external model file identity.');
      const model = database.prepare("SELECT id, path, bytes, sha256, status FROM ai_models WHERE ownership='external'").all()
        .find((row) => {
          try {
            const registered = statSync(row.path, { bigint: true });
            return registered.isFile() && registered.ino !== 0n
              && registered.dev === selectedStat.dev && registered.ino === selectedStat.ino;
          } catch { return false; }
        });
      if (!model) throw new Error('Explicit external file is not registered in this profile.');
      const hash = await sha256(file);
      const actualBytes = Number(selectedStat.size);
      if (!Number.isSafeInteger(actualBytes)) throw new Error('External model size is too large to verify.');
      external = { id: model.id, exists: true, actual_bytes: actualBytes, expected_bytes: model.bytes,
        actual_sha256: hash, expected_sha256: model.sha256, status: model.status,
        matches_registered_size_and_sha256: actualBytes === model.bytes && hash === model.sha256 };
    }
    const result = {
      data_root: root,
      schema_version: scalar('PRAGMA user_version'),
      integrity_check: integrity,
      foreign_key_violations: foreignKeys,
      documents: scalar('SELECT COUNT(*) FROM documents'),
      ai_preferences: database.prepare('SELECT active_model_id, setup_completed, processing_mode, automatic_limit, max_tokens FROM ai_preferences WHERE id=1').get(),
      ai_models: models,
      ai_jobs: jobs,
      ai_results: results,
      explicit_external_model: external,
      interest_counts_by_polarity: database.prepare('SELECT polarity, COUNT(*) AS count FROM personalization_interests GROUP BY polarity ORDER BY polarity').all(),
      proactive_settings: database.prepare('SELECT close_to_background, launch_at_login, start_hidden, scheduler_enabled, scheduler_paused, scheduler_interval_minutes, notifications_enabled, notification_sensitivity, quiet_hours_enabled FROM proactive_settings WHERE id=1').get(),
      proactive_events_by_state: database.prepare('SELECT state, COUNT(*) AS count FROM proactive_events GROUP BY state ORDER BY state').all(),
      proactive_delivery_attempts: scalar('SELECT COUNT(*) FROM proactive_delivery_attempts'),
      notification_activation_tokens: scalar('SELECT COUNT(*) FROM notification_activation_tokens'),
      quality_note: 'Shape checks do not establish grounding or semantic correctness.',
    };
    result.ok = integrity.length === 1 && integrity[0] === 'ok' && foreignKeys === 0
      && results.every((row) => row.inspection.basic_shape_valid)
      && (external === null || external.matches_registered_size_and_sha256);
    return result;
  } finally {
    database.close();
  }
}

try {
  const result = await inspect(options(process.argv.slice(2)));
  console.log(JSON.stringify(result, null, 2));
  if (!result.ok) process.exitCode = 1;
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: error.message }));
  process.exitCode = 1;
}
