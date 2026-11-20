// Browser access e2e: start remotium-host, create a pairing link, run browser.js in Chromium.
//   node test/e2e/run-browser.js [--work <dir>]
const { spawnSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { TestHost } = require('./host');

const i = process.argv.indexOf('--work');
const work = i > 0 ? process.argv[i + 1] : fs.mkdtempSync(path.join(os.tmpdir(), 'remotium-e2e-'));
fs.mkdirSync(work, { recursive: true });

(async () => {
	const host = new TestHost({ work });
	await host.start();
	const { browser, fingerprint } = host.pair('chromium');
	const r = spawnSync(process.execPath, [path.join(__dirname, 'browser.js'), '--link', browser, '--fingerprint', fingerprint, '--folder', host.folder], { stdio: 'inherit' });
	await host.stop();
	process.exit(r.status ?? 1);
})().catch(e => { console.error(e); process.exit(1); });
