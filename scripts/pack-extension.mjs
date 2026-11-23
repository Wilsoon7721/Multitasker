import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const out = join(root, 'dist-extension');
const staging = join(out, 'extension');

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
cpSync(join(root, 'extension'), staging, { recursive: true });

const manifestPath = join(staging, 'manifest.json');
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
// Stores give the extension its own id, so the key that fixes it for unpacked copies has to come out.
delete manifest.key;
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

const zip = join(out, `multitasker-extension-${manifest.version}.zip`);
// Windows' own tar writes zip paths with forward slashes (Compress-Archive writes backslashes the stores can't read);
// it's named in full because a Git Bash on the PATH brings GNU tar, which can't write zips.
const tar = join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe');
execFileSync(tar, ['-a', '-c', '-f', zip, '-C', staging, ...readdirSync(staging)], { stdio: 'inherit' });
console.log(`Packed ${zip}`);
