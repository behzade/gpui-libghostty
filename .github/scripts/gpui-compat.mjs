import { appendFile, copyFile, mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const zed = 'https://api.github.com/repos/zed-industries/zed';
const repository = 'https://github.com/zed-industries/zed';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

async function getJSON(url) {
  const headers = { 'User-Agent': 'gpui-libghostty-compatibility' };
  if (url.startsWith('https://api.github.com/') && process.env.GH_TOKEN) {
    headers.Authorization = `Bearer ${process.env.GH_TOKEN}`;
  }
  const response = await fetch(url, { headers, signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response.json();
}

function commit(value) {
  if (!/^[a-f0-9]{40}$/.test(value)) throw new Error('Expected a full Git commit SHA');
  return value;
}

export async function resolveTargets(get = getJSON) {
  const [release, main, registry] = await Promise.all([
    get(`${zed}/releases/latest`),
    get(`${zed}/commits/main`),
    get('https://crates.io/api/v1/crates/gpui-pre'),
  ]);
  if (release.draft || release.prerelease || !release.tag_name) {
    throw new Error('Expected a stable Zed release tag');
  }
  // Resolve the tag itself, not target_commitish (which may name a moving branch).
  const tagged = await get(`${zed}/commits/${encodeURIComponent(release.tag_name)}`);
  const stable = registry.versions
    .filter(version => !version.yanked && /^\d+\.\d+\.\d+$/.test(version.num))
    .sort((a, b) => b.num.localeCompare(a.num, 'en', { numeric: true }));
  if (!stable.length) throw new Error('No non-yanked stable gpui-pre release');
  return [
    { channel: 'zed-release', rev: commit(tagged.sha), tag: release.tag_name },
    { channel: 'zed-main', rev: commit(main.sha) },
    { channel: 'gpui-pre', version: stable[0].num },
  ];
}

function consumerManifest(target) {
  let dependency;
  if (target.channel === 'gpui-pre' && /^\d+\.\d+\.\d+$/.test(target.version)) {
    dependency = `package = "gpui-pre", version = "=${target.version}"`;
  } else if (['zed-release', 'zed-main'].includes(target.channel)) {
    dependency = `package = "gpui", git = "${repository}", rev = "${commit(target.rev)}"`;
  } else {
    throw new Error('Invalid compatibility target');
  }
  return `[package]
name = "gpui-compat-${target.channel}"
version = "0.0.0"
edition = "2024"
publish = false

# Resolve independently of the pinned workspace test dependencies.
[workspace]

[dependencies]
chosen-ui = { ${dependency}, default-features = false }
terminal-library = { package = "gpui-libghostty", path = "../../../crates/gpui-ghostty" }
editor-library = { package = "gpui-neovim", path = "../../../crates/gpui-neovim" }

[lints.rust]
unsafe_code = "forbid"
`;
}

async function main() {
  if (process.argv[2] === 'resolve') {
    const targets = await resolveTargets();
    console.log(JSON.stringify(targets, null, 2));
    if (process.env.GITHUB_OUTPUT) {
      await appendFile(process.env.GITHUB_OUTPUT, `targets=${JSON.stringify(targets)}\n`);
    }
  } else if (process.argv[2] === 'prepare') {
    const target = JSON.parse(process.env.TARGET_JSON);
    const manifest = consumerManifest(target);
    const directory = join(root, 'target/gpui-compat', target.channel);
    await mkdir(join(directory, 'src'), { recursive: true });
    await writeFile(join(directory, 'Cargo.toml'), manifest);
    for (const file of ['lib.rs', 'bindings.rs']) {
      await copyFile(join(root, 'tests/gpui-compat/src', file), join(directory, 'src', file));
    }
    await writeFile(join(directory, 'resolved.json'), JSON.stringify({
      ...target,
      library_sha: process.env.GITHUB_SHA,
      runner_os: process.env.RUNNER_OS,
      consumer_path: `target/gpui-compat/${target.channel}`,
    }, null, 2) + '\n');
  } else {
    throw new Error('Usage: gpui-compat.mjs resolve|prepare');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await main();
}
