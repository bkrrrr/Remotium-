// Browser access through remotium-host, in headless Chromium:
//  1. the host presents the certificate the pairing link announced (fingerprint check);
//  2. pair this browser through the page;
//  3. the VSCodium workbench loads (its WebSocket goes through the host's proxy);
//  4. edit README.md in the editor, save, and find the change on disk.
//   node test/e2e/browser.js --link <https://host/pair?code=…> --fingerprint <sha256 hex> --folder <dir on disk>
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');
const tls = require('tls');
const net = require('net');

function arg(name, def) {
	const i = process.argv.indexOf(`--${name}`);
	return i > 0 ? process.argv[i + 1] : def;
}

function peerFingerprint(url) {
	const u = new URL(url);
	return new Promise((resolve, reject) => {
		const s = tls.connect({ host: u.hostname, port: Number(u.port || 443), ...(net.isIP(u.hostname.replace(/^\[|\]$/g, '')) ? {} : { servername: u.hostname }), rejectUnauthorized: false }, () => {
			const fp = s.getPeerCertificate().fingerprint256.replace(/:/g, '').toLowerCase();
			s.end();
			resolve(fp);
		});
		s.on('error', reject);
	});
}

(async () => {
	const link = arg('link');
	const expected = (arg('fingerprint') || '').replace(/:/g, '').toLowerCase();
	const folder = arg('folder');
	const result = { ok: false, steps: [] };
	const step = s => { result.steps.push(s); console.log('[browser]', s); };
	let browser;
	try {
		if (expected) {
			const fp = await peerFingerprint(link);
			if (fp !== expected) throw new Error(`certificate fingerprint ${fp} != ${expected}`);
			step('certificate matches the pairing fingerprint');
		}
		browser = await chromium.launch();
		const context = await browser.newContext({ ignoreHTTPSErrors: !!expected });
		const page = await context.newPage();
		await page.goto(link);
		await page.getByRole('button', { name: 'Pair' }).click();
		await page.waitForSelector('.monaco-workbench', { timeout: 180_000 });
		step('workbench loaded');
		const readme = page.getByRole('treeitem', { name: /README\.md/ });
		await readme.waitFor({ timeout: 120_000 });
		step('explorer lists README.md (remote file system over the proxied WebSocket)');
		await readme.dblclick();
		const editor = page.locator('.editor-instance .monaco-editor .view-lines').first();
		await editor.waitFor({ timeout: 60_000 });
		await editor.click();
		const marker = `edited in the browser ${Date.now()}`;
		await page.keyboard.press('ControlOrMeta+End');
		await page.keyboard.press('Enter');
		await page.keyboard.type(marker);
		await page.keyboard.press('ControlOrMeta+S');
		const file = path.join(folder, 'README.md');
		const end = Date.now() + 30_000;
		while (!fs.readFileSync(file, 'utf8').includes(marker)) {
			if (Date.now() > end) throw new Error('the edit did not reach the disk');
			await new Promise(r => setTimeout(r, 300));
		}
		step('edit saved to disk');
		result.ok = true;
	} catch (e) {
		result.error = String(e && e.stack || e);
		console.error('[browser] FAILED', e);
	} finally {
		await browser?.close();
	}
	console.log(JSON.stringify(result, null, 2));
	process.exit(result.ok ? 0 : 1);
})();
