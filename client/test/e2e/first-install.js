// The first-install experience with the packaged extension, as a user gets it:
//  1. install the VSIX into a fresh profile, start VSCodium WITHOUT --enable-proposed-api,
//     let the client add itself to argv.json (what its "Set Up" button does);
//  2. restart VSCodium (still without the flag), pair with a remotium-host through its link
//     and run the smoke suite.
// Portable mode keeps argv.json inside the temp profile, so the real ~/.vscode-oss is untouched.
//   node test/e2e/first-install.js [--vsix <file|store id>] [--work <dir>]
const { spawnSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

function arg(name, def) {
	const i = process.argv.indexOf(`--${name}`);
	return i > 0 ? process.argv[i + 1] : def;
}

const vsix = arg('vsix', path.join(__dirname, '..', '..', 'remotium-client.vsix'));
const work = arg('work') || fs.mkdtempSync(path.join(os.tmpdir(), 'remotium-e2e-'));
// Short path: the editor's IPC socket ends up in here (macOS limits socket paths).
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'ot-'));
const portable = path.join(profile, 'p');
fs.mkdirSync(portable, { recursive: true });
fs.writeFileSync(path.join(portable, 'argv.json'),
	'// This configuration file allows you to pass permanent command line arguments to VS Code.\n{\n\t// Use software rendering instead of hardware accelerated rendering.\n\t// "disable-hardware-acceleration": true,\n\t"enable-crash-reporter": false\n}\n');

function run(name, extra) {
	console.log(`== ${name}`);
	const r = spawnSync(process.execPath, [
		path.join(__dirname, 'run-host.js'), '--vsix', vsix, '--no-proposed-flag',
		'--portable', portable, '--profile', profile, '--work', path.join(work, name), ...extra,
	], { stdio: 'inherit' });
	return r.status === 0;
}

let ok = run('1-setup', ['--suite', 'setup', '--timeout', '120000']);
if (ok) {
	const argv = fs.readFileSync(path.join(portable, 'argv.json'), 'utf8');
	console.log(`argv.json after setup:\n${argv}`);
	ok = argv.includes('"remotium.client"') && argv.includes('// Use software rendering');
	if (!ok) console.error('argv.json lacks the entry or lost its comments');
}
if (ok) ok = run('2-launch', ['--timeout', '180000']);
fs.rmSync(profile, { recursive: true, force: true });
console.log(ok ? 'first install: OK' : 'first install: FAILED');
process.exit(ok ? 0 : 1);
