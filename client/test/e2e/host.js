// Start/stop a real remotium-host for tests, and create pairing links.
const { spawn, spawnSync } = require('child_process');
const fs = require('fs');
const net = require('net');
const os = require('os');
const path = require('path');
const tls = require('tls');

const repo = path.resolve(__dirname, '..', '..', '..');

function freePort() {
	return new Promise(resolve => {
		const s = net.createServer().listen(0, '127.0.0.1', () => {
			const p = s.address().port;
			s.close(() => resolve(p));
		});
	});
}

function healthy(port) {
	return new Promise(resolve => {
		const s = tls.connect({ host: '127.0.0.1', port, rejectUnauthorized: false }, () => {
			s.write('GET /remotium/healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n');
		});
		let data = '';
		s.on('data', d => (data += d));
		s.on('end', () => resolve(data.startsWith('HTTP/1.1 200')));
		s.on('error', () => resolve(false));
	});
}

class TestHost {
	/**
	 * @param {{ bin?: string, data?: string, folder?: string, work: string }} o
	 * `data` is persistent by default, so the VSCodium server is downloaded only once.
	 */
	constructor(o) {
		this.bin = o.bin || process.env.REMOTIUM_HOST_BIN || path.join(repo, 'target', 'debug', 'remotium-host');
		this.data = o.data || path.join(repo, 'client', '.e2e', 'host-data');
		this.work = o.work;
		this.folder = o.folder;
		if (!this.folder) {
			this.folder = fs.mkdtempSync(path.join(os.tmpdir(), 'remotium-ws-'));
			fs.writeFileSync(path.join(this.folder, 'README.md'), '# demo\n');
		}
	}

	async start() {
		this.port ??= await freePort();
		const log = fs.openSync(path.join(this.work, 'host.log'), 'a');
		this.proc = spawn(this.bin, ['--data-dir', this.data, '--listen', `127.0.0.1:${this.port}`, '--folder', this.folder], { stdio: ['ignore', log, log] });
		for (let i = 0; i < 100; i++) {
			if (await healthy(this.port)) return;
			await new Promise(r => setTimeout(r, 200));
		}
		throw new Error(`remotium-host didn't come up (see ${this.work}/host.log)`);
	}

	stop() {
		return new Promise(resolve => {
			if (!this.proc || this.proc.exitCode !== null) return resolve();
			this.proc.once('exit', () => resolve());
			this.proc.kill('SIGTERM');
		});
	}

	async restart() {
		await this.stop();
		await this.start();
	}

	/** { browser, vscodium, fingerprint } from `remotium-host pair`. */
	pair(name = 'e2e') {
		const out = spawnSync(this.bin, ['--data-dir', this.data, 'pair', '--name', name], { encoding: 'utf8' }).stdout;
		const browser = (out.match(/Browser:\s+(\S+)/) || [])[1];
		const vscodium = (out.match(/vscodium:\/\/\S+/) || [])[0];
		const fingerprint = (out.match(/[?&]fp=([0-9a-f]{64})/) || [])[1];
		if (!browser || !vscodium) throw new Error(`no pairing link in:\n${out}`);
		return { browser, vscodium, fingerprint };
	}
}

module.exports = { TestHost };
