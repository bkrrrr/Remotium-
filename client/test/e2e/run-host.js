// Runs a real remotium-host on a free local port, pairs VSCodium with it and runs a suite.
//   node test/e2e/run-host.js [--suite smoke|phase3|setup] [launch.js options…]
// Environment: REMOTIUM_HOST_BIN (default target/debug/remotium-host); the host's data dir
// is client/.e2e/host-data so the VSCodium server is downloaded only once.
// The suite can ask for a host restart (request-restart-host in the work dir).
const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { TestHost } = require('./host');

const i = process.argv.indexOf('--work');
const work = i > 0 ? process.argv[i + 1] : fs.mkdtempSync(path.join(os.tmpdir(), 'remotium-e2e-'));
fs.mkdirSync(work, { recursive: true });
const passthrough = process.argv.slice(2).filter((a, j, all) => a !== '--work' && all[j - 1] !== '--work');

(async () => {
	const host = new TestHost({ work });
	await host.start();
	const { vscodium } = host.pair('e2e');
	console.log(`host on 127.0.0.1:${host.port}, folder ${host.folder}`);

	const handled = new Set();
	const control = setInterval(async () => {
		for (const f of fs.readdirSync(work)) {
			const m = /^request-(.+)$/.exec(f);
			if (!m || handled.has(m[1])) continue;
			handled.add(m[1]);
			console.log(`[runner] ${m[1]}`);
			let status = 2;
			if (m[1] === 'restart-host') {
				try {
					await host.restart();
					status = 0;
				} catch (e) {
					console.error(e);
					status = 1;
				}
			}
			fs.writeFileSync(path.join(work, `done-${m[1]}`), String(status));
		}
	}, 500);

	// Asynchronous, so restart requests are handled while the suite runs.
	const child = spawn(process.execPath, [path.join(__dirname, 'launch.js'), '--pair-link', vscodium, '--work', work, ...passthrough], { stdio: 'inherit' });
	const status = await new Promise(resolve => child.on('exit', code => resolve(code ?? 1)));
	clearInterval(control);
	await host.stop();
	process.exit(status);
})().catch(e => { console.error(e); process.exit(1); });
