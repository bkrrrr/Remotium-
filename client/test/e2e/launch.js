// Launch an isolated VSCodium with the extension and run a test file in it.
//   node test/e2e/launch.js [--codium <VSCodium executable>] [--folder-uri <uri>] [--tests smoke.js]
//                           [--settings <json>] [--work <dir>] [--profile <dir>] [--keep-profile]
//                           [--vsix <file|url>] [--no-proposed-flag] [--portable <dir>]
// --profile reuses a user-data dir across runs.
// --vsix installs that package (file, URL, or a store id like remotium.client@0.2.0), as users
// would, instead of loading the extension from source.
// --no-proposed-flag leaves out --enable-proposed-api, so argv.json has to provide it.
// --portable runs the editor in portable mode with that data dir. It then reads <dir>/argv.json
// instead of ~/.vscode-oss/argv.json, so argv.json handling is tested without touching the real
// one. (Never change HOME for this: on macOS the editor then can't find the login keychain.)
// --launch-link <uri> runs the server flow instead of a test file: the helper extension opens
// the link in a local window, then runs smoke.js in the remote window that opens.
const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

function arg(name, def) {
	const i = process.argv.indexOf(`--${name}`);
	return i > 0 ? process.argv[i + 1] : def;
}

const defaultCodium = process.platform === 'darwin'
	? '/Applications/VSCodium.app/Contents/MacOS/VSCodium'
	: 'codium';
const codium = arg('codium', process.env.VSCODIUM_PATH || defaultCodium);
const folderUri = arg('folder-uri');
const work = arg('work') || fs.mkdtempSync(path.join(os.tmpdir(), 'remotium-e2e-'));
fs.mkdirSync(work, { recursive: true });
const settings = JSON.parse(arg('settings', '{}'));
const timeoutMs = Number(arg('timeout', '180000'));
const tests = path.resolve(__dirname, arg('tests', 'smoke.js'));
const launchLink = arg('launch-link');

// The editor's IPC socket lives in the user data dir and macOS limits socket paths to 103
// characters, so the profile always goes into a short temp dir.
const profile = arg('profile') || fs.mkdtempSync(path.join(os.tmpdir(), 'ot-'));
const keepProfile = process.argv.includes('--keep-profile') || !!arg('profile');
// In portable mode the editor always uses <portable>/user-data, whatever --user-data-dir says.
const portableDir = arg('portable');
const userData = portableDir ? path.join(path.resolve(portableDir), 'user-data') : path.join(profile, 'ud');
fs.mkdirSync(path.join(userData, 'User'), { recursive: true });
fs.writeFileSync(path.join(userData, 'User', 'settings.json'), JSON.stringify({
	'security.workspace.trust.enabled': false,
	'extensions.autoCheckUpdates': false,
	'extensions.autoUpdate': false,
	'update.mode': 'none',
	'telemetry.telemetryLevel': 'off',
	// Publishers allowed by this policy count as trusted, so installing an extension (the
	// phase 3 suite installs ms-python.debugpy) never shows the publisher trust dialog.
	'extensions.allowed': { '*': true },
	...settings,
	...(launchLink ? {
		'remotiumE2e.launchLink': launchLink,
		'remotiumE2e.resultFile': path.join(work, 'result.json'),
		'remotiumE2e.suite': arg('suite', 'smoke'),
		'remotiumE2e.controlDir': work,
		'remotiumE2e.only': arg('only', ''),
	} : {}),
}, null, 2));
const resultFile = path.join(work, 'result.json');
try { fs.unlinkSync(resultFile); } catch { }

const extRoot = path.resolve(__dirname, '..', '..');
const vsix = arg('vsix');
const portable = arg('portable');
if (portable) fs.mkdirSync(portable, { recursive: true });
const childEnv = { ...process.env, SMOKE_RESULT: resultFile, ...(portable ? { VSCODE_PORTABLE: path.resolve(portable) } : {}) };
// When launched from inside another VS Code, this would make the editor run as plain Node.
delete childEnv.ELECTRON_RUN_AS_NODE;

/** The `codium` command-line script next to the editor executable. */
function cliPath() {
	if (process.env.VSCODIUM_CLI) return process.env.VSCODIUM_CLI;
	const dir = path.dirname(codium);
	const candidates = process.platform === 'darwin'
		? [path.join(dir, '..', 'Resources', 'app', 'bin', 'codium')]
		: process.platform === 'win32'
			? [path.join(dir, 'bin', 'codium.cmd')]
			: [path.join(dir, 'bin', 'codium')];
	return candidates.find(c => fs.existsSync(c)) || 'codium';
}

async function installVsix() {
	let file = vsix;
	// An extension id (e.g. remotium.client@0.2.0) installs from the gallery (Open VSX).
	const fromStore = /^[a-z0-9-]+\.[a-z0-9-]+(@[\w.-]+)?$/i.test(vsix) && !fs.existsSync(vsix);
	if (/^https?:\/\//.test(vsix)) {
		const res = await fetch(vsix);
		if (!res.ok) throw new Error(`download ${vsix}: HTTP ${res.status}`);
		file = path.join(work, 'remotium-client.vsix');
		fs.writeFileSync(file, Buffer.from(await res.arrayBuffer()));
	}
	const r = require('child_process').spawnSync(cliPath(), [
		'--user-data-dir', userData, '--extensions-dir', path.join(profile, 'ext'),
		'--install-extension', fromStore ? vsix : path.resolve(file), '--force',
	], { env: childEnv, encoding: 'utf8', shell: process.platform === 'win32' });
	console.log(`[install] ${(r.stdout || '').trim()} ${(r.stderr || '').trim()}`.trim());
	if (r.status !== 0) throw new Error(`installing ${file} failed (exit ${r.status})`);
}

const args = [
	'--user-data-dir', userData,
	'--extensions-dir', path.join(profile, 'ext'),
	// Otherwise the instance shares ~/.vscode-oss-shared with a running everyday VSCodium and
	// can hang at startup waiting for it.
	'--shared-data-dir', path.join(profile, 'shared'),
	...(vsix ? [] : ['--extensionDevelopmentPath', extRoot]),
	...(launchLink
		? ['--extensionDevelopmentPath', path.join(__dirname, 'helper')]
		: ['--extensionTestsPath', tests]),
	...(process.argv.includes('--no-proposed-flag') ? [] : ['--enable-proposed-api', 'remotium.client']),
	'--disable-workspace-trust',
	'--skip-welcome', '--skip-release-notes', '--disable-gpu',
	...(folderUri ? ['--folder-uri', folderUri] : []),
	// No keyring on headless Linux (CI): keep secrets in the profile instead.
	...(process.platform === 'linux' ? ['--password-store=basic'] : []),
	// Extra editor flags, e.g. --no-sandbox on CI runners that restrict user namespaces.
	...(process.env.E2E_CODIUM_ARGS ? process.env.E2E_CODIUM_ARGS.split(/\s+/).filter(Boolean) : []),
];
(async () => {
if (vsix) await installVsix();
console.log(`launching ${codium}`);
const child = spawn(codium, args, { env: childEnv, stdio: ['ignore', 'pipe', 'pipe'] });
const logFile = fs.createWriteStream(path.join(work, 'codium.log'));
child.stdout.pipe(logFile);
child.stderr.pipe(logFile);
child.stdout.on('data', d => { const s = d.toString(); if (/\[(smoke|phase3)\]/.test(s)) process.stdout.write(s); });
// Requests from the in-window suite (e.g. restart the server), answered by --on-request.
const onRequest = arg('on-request');
const handled = new Set();
const control = onRequest && setInterval(() => {
	for (const f of fs.readdirSync(work)) {
		const m = /^request-(.+)$/.exec(f);
		if (!m || handled.has(m[1])) continue;
		handled.add(m[1]);
		console.log(`[runner] ${m[1]}`);
		const r = require('child_process').spawnSync(onRequest, [m[1]], { stdio: 'inherit', shell: false });
		console.log(`[runner] ${m[1]} → exit ${r.status}`);
		fs.writeFileSync(path.join(work, `done-${m[1]}`), String(r.status));
	}
}, 500);
// In helper mode the app may not exit on its own if something hangs: stop once a result exists.
const poll = launchLink && setInterval(() => { if (fs.existsSync(resultFile)) setTimeout(() => child.kill(), 3000); }, 1000);
const timer = setTimeout(() => { console.error('timeout'); child.kill(); }, timeoutMs);
child.on('exit', code => {
	clearTimeout(timer);
	if (poll) clearInterval(poll);
	if (control) clearInterval(control);
	if (!keepProfile) fs.rmSync(profile, { recursive: true, force: true });
	let result;
	try { result = JSON.parse(fs.readFileSync(resultFile, 'utf8')); } catch { }
	console.log(JSON.stringify(result ?? { ok: false, error: 'no result file' }, null, 2));
	console.log(`exit ${code}, logs in ${work}`);
	process.exit(result?.ok ? 0 : 1);
});
})().catch(e => { console.error(e); process.exit(1); });
