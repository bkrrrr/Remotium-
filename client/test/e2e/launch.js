// Launch an isolated VSCodium with the extension under development and run a test file in it.
//   node test/e2e/launch.js [--codium <VSCodium executable>] [--folder-uri <uri>] [--tests smoke.js]
//                           [--settings <json>] [--work <dir>] [--profile <dir>] [--keep-profile]
// --profile reuses a user-data dir across runs.
// --launch-link <uri> runs the broker flow instead of a test file: the helper extension opens
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
const work = arg('work', fs.mkdtempSync(path.join(os.tmpdir(), 'opentunnel-e2e-')));
fs.mkdirSync(work, { recursive: true });
const settings = JSON.parse(arg('settings', '{}'));
const timeoutMs = Number(arg('timeout', '180000'));
const tests = path.resolve(__dirname, arg('tests', 'smoke.js'));
const launchLink = arg('launch-link');

// The editor's IPC socket lives in the user data dir and macOS limits socket paths to 103
// characters, so the profile always goes into a short temp dir.
const profile = arg('profile') || fs.mkdtempSync(path.join(os.tmpdir(), 'ot-'));
const keepProfile = process.argv.includes('--keep-profile') || !!arg('profile');
const userData = path.join(profile, 'ud');
fs.mkdirSync(path.join(userData, 'User'), { recursive: true });
fs.writeFileSync(path.join(userData, 'User', 'settings.json'), JSON.stringify({
	'security.workspace.trust.enabled': false,
	'extensions.autoCheckUpdates': false,
	'extensions.autoUpdate': false,
	'update.mode': 'none',
	'telemetry.telemetryLevel': 'off',
	...settings,
	...(launchLink ? { 'opentunnelE2e.launchLink': launchLink, 'opentunnelE2e.resultFile': path.join(work, 'result.json') } : {}),
}, null, 2));
const resultFile = path.join(work, 'result.json');
try { fs.unlinkSync(resultFile); } catch { }

const extRoot = path.resolve(__dirname, '..', '..');
const args = [
	'--user-data-dir', userData,
	'--extensions-dir', path.join(profile, 'ext'),
	'--extensionDevelopmentPath', extRoot,
	...(launchLink
		? ['--extensionDevelopmentPath', path.join(__dirname, 'helper')]
		: ['--extensionTestsPath', tests]),
	'--enable-proposed-api', 'opentunnel.client',
	'--disable-workspace-trust',
	'--skip-welcome', '--skip-release-notes', '--disable-gpu',
	...(folderUri ? ['--folder-uri', folderUri] : []),
];
console.log(`launching ${codium}`);
const env = { ...process.env, SMOKE_RESULT: resultFile };
// When launched from inside another VS Code, this would make the editor run as plain Node.
delete env.ELECTRON_RUN_AS_NODE;
const child = spawn(codium, args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
const logFile = fs.createWriteStream(path.join(work, 'codium.log'));
child.stdout.pipe(logFile);
child.stderr.pipe(logFile);
child.stdout.on('data', d => { const s = d.toString(); if (s.includes('[smoke]')) process.stdout.write(s); });
// In helper mode the app may not exit on its own if something hangs: stop once a result exists.
const poll = launchLink && setInterval(() => { if (fs.existsSync(resultFile)) setTimeout(() => child.kill(), 3000); }, 1000);
const timer = setTimeout(() => { console.error('timeout'); child.kill(); }, timeoutMs);
child.on('exit', code => {
	clearTimeout(timer);
	if (poll) clearInterval(poll);
	if (!keepProfile) fs.rmSync(profile, { recursive: true, force: true });
	let result;
	try { result = JSON.parse(fs.readFileSync(resultFile, 'utf8')); } catch { }
	console.log(JSON.stringify(result ?? { ok: false, error: 'no result file' }, null, 2));
	console.log(`exit ${code}, logs in ${work}`);
	process.exit(result?.ok ? 0 : 1);
});
