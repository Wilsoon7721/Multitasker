import { execFileSync } from 'node:child_process';
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
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

// Signed for the Chrome Web Store's verified uploads, with the key CI passes in or the one kept in .secrets.
const localKey = join(root, '.secrets', 'extension-signing.pem');
const signingKey =
  process.env.MULTITASKER_EXTENSION_SIGNING_KEY || (existsSync(localKey) ? readFileSync(localKey, 'utf8') : '');
if (!signingKey.trim()) {
  console.log('No signing key, so only the zip was made.');
} else {
  const browser = [
    process.env.CHROME_PATH,
    join(process.env.ProgramFiles ?? 'C:\\Program Files', 'Google', 'Chrome', 'Application', 'chrome.exe'),
    join(
      process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)',
      'Microsoft',
      'Edge',
      'Application',
      'msedge.exe',
    ),
  ].find((path) => path && existsSync(path));
  if (!browser) throw new Error('Signing needs Chrome or Edge; set CHROME_PATH to one.');

  // The key only touches disk for as long as packing takes, outside the project. A profile of its own keeps a browser
  // that's already open from taking the job and returning before the package is written.
  const scratch = mkdtempSync(join(tmpdir(), 'multitasker-sign-'));
  const keyFile = join(scratch, 'key.pem');
  writeFileSync(keyFile, signingKey);
  try {
    execFileSync(
      browser,
      [
        `--pack-extension=${staging}`,
        `--pack-extension-key=${keyFile}`,
        `--user-data-dir=${join(scratch, 'profile')}`,
        '--no-message-box',
      ],
      { stdio: 'inherit' },
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  // The browser writes the package beside the folder it packed, named after it.
  const crx = join(out, `multitasker-extension-${manifest.version}.crx`);
  renameSync(`${staging}.crx`, crx);
  console.log(`Signed ${crx}`);
}
