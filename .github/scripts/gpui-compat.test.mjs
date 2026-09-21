import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveTargets } from './gpui-compat.mjs';

const main = 'a'.repeat(40);
const release = 'b'.repeat(40);

function responses(overrides = {}) {
  return {
    'releases/latest': { tag_name: 'v1.2.3', target_commitish: 'main' },
    'commits/main': { sha: main },
    'commits/v1.2.3': { sha: release },
    'gpui-pre': { versions: [
      { num: '0.3.9', yanked: false },
      { num: '0.3.10', yanked: false },
      { num: '0.4.0', yanked: true },
      { num: '0.5.0-rc.1', yanked: false },
    ] },
    ...overrides,
  };
}

function reader(data) {
  return async url => {
    const key = url.replace(/^.*\/zed\//, '').replace(/^.*\/crates\//, '');
    assert.ok(Object.hasOwn(data, key), `Unexpected request: ${url}`);
    return data[key];
  };
}

test('freezes tag and main independently and selects the newest non-yanked stable crate', async () => {
  assert.deepEqual(await resolveTargets(reader(responses())), [
    { channel: 'zed-release', tag: 'v1.2.3', rev: release },
    { channel: 'zed-main', rev: main },
    { channel: 'gpui-pre', version: '0.3.10' },
  ]);
});

test('fails closed for invalid revisions or no stable release', async () => {
  await assert.rejects(resolveTargets(reader(responses({ 'commits/main': { sha: 'main' } }))), /SHA/);
  await assert.rejects(resolveTargets(reader(responses({ 'gpui-pre': { versions: [] } }))), /stable/);
  await assert.rejects(resolveTargets(reader(responses({
    'releases/latest': { tag_name: 'preview', prerelease: true },
  }))), /stable/);
});
