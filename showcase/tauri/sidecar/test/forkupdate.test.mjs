import test from 'node:test';
import assert from 'node:assert/strict';
import { parseBuild, isNewer, checkFork } from '../forkupdate.mjs';

test('tags and versions are read, the rest is not', () => {
  assert.deepEqual(parseBuild('v0.1.46-linux.3'), [0, 1, 46, 3]);
  assert.deepEqual(parseBuild('0.1.46-linux.12'), [0, 1, 46, 12]);
  assert.deepEqual(parseBuild('v0.1.46-linux'), [0, 1, 46, 0]);
  assert.deepEqual(parseBuild('0.1.46'), [0, 1, 46, 0]);
  assert.equal(parseBuild('nightly'), null);
  assert.equal(parseBuild(''), null);
});
test('newer means a later build, numerically, or a later version', () => {
  assert.equal(isNewer('v0.1.46-linux.10', '0.1.46-linux.9'), true);
  assert.equal(isNewer('0.1.46-linux.2', '0.1.46-linux.2'), false);
  assert.equal(isNewer('0.1.46-linux.1', '0.1.46'), true);
  assert.equal(isNewer('0.1.47-linux.1', '0.1.46-linux.20'), true);
  assert.equal(isNewer('0.1.45-linux.9', '0.1.46'), false);
  assert.equal(isNewer('weird', '0.1.46'), false);
});
test('the check picks the newest published release and compares it to the running build', async () => {
  const list = [{ tag_name: 'v0.1.46-linux.1', html_url: 'u1' }, { tag_name: 'v0.1.46-linux.3', html_url: 'u3' }, { tag_name: 'v0.1.46-linux.4', draft: true }, { tag_name: 'junk' }];
  const f = async () => ({ ok: true, json: async () => list });
  assert.deepEqual(await checkFork('a/b', '0.1.46-linux.1', f), { state: 'available', version: '0.1.46-linux.3' });
  assert.deepEqual(await checkFork('a/b', '0.1.46-linux.3', f), { state: 'none' });
  await assert.rejects(checkFork('a/b', '0.1.46', async () => ({ ok: false, status: 403 })), /403/);
});
