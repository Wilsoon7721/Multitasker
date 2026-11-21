import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

// Usage: npm run set-version -- 0.2.0 (the workspace Cargo.toml is what the installer and updates read).
const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) {
  console.error('Give a version like 0.2.0.');
  process.exit(1);
}

const root = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const update = (file, change) => {
  const path = join(root, file);
  writeFileSync(path, change(readFileSync(path, 'utf8')));
  console.log(`${file} → ${version}`);
};

update('Cargo.toml', (text) => text.replace(/^version = "[^"]*"/m, `version = "${version}"`));
for (const file of ['package.json', 'extension/manifest.json']) {
  update(file, (text) => text.replace(/"version": "[^"]*"/, `"version": "${version}"`));
}
