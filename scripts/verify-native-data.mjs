// Read-only inspection of an explicitly selected native validation profile.
// node --experimental-sqlite scripts/verify-native-data.mjs --data-root <path>
//   --compare-source <path> [--expected-saved-doc10 0|1]
import { createHash } from 'node:crypto';
import { createReadStream, realpathSync, statSync } from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

function options(args) {
  const values = {};
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i];
    if (!['--data-root', '--compare-source', '--expected-saved-doc10'].includes(key)
      || values[key] !== undefined || !args[i + 1] || args[i + 1].startsWith('--')) {
      throw new Error(`Invalid or duplicate argument: ${key}`);
    }
    values[key] = args[i + 1];
  }
  if (!values['--data-root'] || !values['--compare-source']) {
    throw new Error('Both --data-root and --compare-source are required.');
  }
  for (const key of ['--data-root', '--compare-source']) {
    if (!path.isAbsolute(values[key])) throw new Error(`${key} must be absolute.`);
  }
  if (values['--expected-saved-doc10'] !== undefined
    && !['0', '1'].includes(values['--expected-saved-doc10'])) {
    throw new Error('--expected-saved-doc10 must be 0 or 1.');
  }
  return values;
}

function localResource(root, name) {
  if (typeof name !== 'string' || !name.startsWith('resources/') || name.includes('\\')
    || name.split('/').some((part) => !part || part === '.' || part === '..' || part.includes(':'))) {
    throw new Error('Invalid resource path in the database.');
  }
  const target = realpathSync(path.join(root, ...name.split('/')));
  const relative = path.relative(root, target);
  if (!relative || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new Error('Resource resolves outside the selected data root.');
  }
  if (!statSync(target).isFile()) throw new Error('Resource is not a regular file.');
  return target;
}

async function fileDigest(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

async function verify(values) {
  const root = realpathSync(values['--data-root']);
  const source = realpathSync(values['--compare-source']);
  const database = new DatabaseSync(path.join(root, 'rhythm.sqlite3'), { readOnly: true });
  let baseline;
  try {
    baseline = new DatabaseSync(path.join(source, 'rhythm.sqlite3'), { readOnly: true });
    const scalar = (db, sql) => Object.values(db.prepare(sql).get())[0];
    const integrity = database.prepare('PRAGMA integrity_check').all().map((r) => r.integrity_check);
    const foreignKeys = database.prepare('PRAGMA foreign_key_check').all().length;
    const revisionsSql = 'SELECT id, document_id, content_hash FROM document_revisions ORDER BY id';
    const revisions = database.prepare(revisionsSql).all();
    const sourceRevisions = baseline.prepare(revisionsSql).all();
    const revisionsMatch = JSON.stringify(revisions) === JSON.stringify(sourceRevisions);
    const referenceSql = "SELECT id, local_path, bytes, sha256 FROM resources WHERE status='downloaded' ORDER BY id";
    const references = database.prepare(referenceSql).all();
    const sourceReferences = baseline.prepare(referenceSql).all();
    const referencesMatch = JSON.stringify(references) === JSON.stringify(sourceReferences);
    const verified = new Map();
    const referenceResults = [];
    let logicalBytes = 0;
    let matched = 0;
    for (const row of references) {
      const file = localResource(root, row.local_path);
      if (!verified.has(file)) verified.set(file, { bytes: statSync(file).size, sha256: await fileDigest(file) });
      const actual = verified.get(file);
      logicalBytes += row.bytes;
      const matches = actual.bytes === row.bytes && actual.sha256 === row.sha256;
      if (matches) matched++;
      referenceResults.push({
        id: row.id, local_path: row.local_path, expected_bytes: row.bytes,
        actual_bytes: actual.bytes, expected_sha256: row.sha256, actual_sha256: actual.sha256, matches,
      });
    }
    const savedRow = database.prepare('SELECT saved FROM document_state WHERE document_id=10').get();
    const saved = savedRow?.saved ?? null;
    const expectedSaved = values['--expected-saved-doc10'];
    const savedMatches = expectedSaved === undefined ? null : saved === Number(expectedSaved);
    const documents = scalar(database, 'SELECT COUNT(*) FROM documents');
    const sourceDocuments = scalar(baseline, 'SELECT COUNT(*) FROM documents');
    const result = {
      data_root: root,
      compare_source: source,
      integrity_check: integrity,
      foreign_key_violations: foreignKeys,
      schema_version: scalar(database, 'PRAGMA user_version'),
      documents,
      source_documents: sourceDocuments,
      revisions: revisions.length,
      source_revisions_equal: revisionsMatch,
      notification_activation_tokens: scalar(database, 'SELECT COUNT(*) FROM notification_activation_tokens'),
      downloaded_references: references.length,
      source_references_equal: referencesMatch,
      references_with_matching_size_and_sha256: matched,
      reference_results: referenceResults,
      logical_resource_bytes: logicalBytes,
      distinct_files: verified.size,
      distinct_file_bytes: [...verified.values()].reduce((sum, item) => sum + item.bytes, 0),
      document_10_saved: saved,
      expected_document_10_saved: expectedSaved === undefined ? null : Number(expectedSaved),
      expected_document_10_saved_matches: savedMatches,
    };
    result.ok = integrity.length === 1 && integrity[0] === 'ok' && foreignKeys === 0
      && documents === sourceDocuments && revisionsMatch && referencesMatch
      && matched === references.length && savedMatches !== false;
    return result;
  } finally {
    baseline?.close();
    database.close();
  }
}

try {
  const result = await verify(options(process.argv.slice(2)));
  console.log(JSON.stringify(result, null, 2));
  if (!result.ok) process.exitCode = 1;
} catch (error) {
  console.error(JSON.stringify({ ok: false, error: error.message }));
  process.exitCode = 1;
}
