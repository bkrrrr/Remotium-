// E2E against a running broker (by default the compose stack on http://localhost:8080):
// gets a launch link from the dev-identity API, then runs the real launch flow in VSCodium.
//   node test/e2e/run-broker.js [--broker http://localhost:8080] [--env demo] [--project demo]
const { spawnSync } = require('child_process');
const path = require('path');

function arg(name, def) {
	const i = process.argv.indexOf(`--${name}`);
	return i > 0 ? process.argv[i + 1] : def;
}

(async () => {
	const broker = arg('broker', process.env.OPENTUNNEL_BROKER || 'http://localhost:8080');
	const envId = arg('env', 'demo');
	const project = arg('project', 'demo');
	const res = await fetch(`${broker}/api/v1/launch-tickets`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ envId, project }),
	});
	if (!res.ok) {
		console.error(`launch ticket: HTTP ${res.status} ${await res.text()}`);
		process.exit(1);
	}
	const { uri } = await res.json();
	const extra = process.argv.slice(2).filter((a, i, all) => !['--broker', '--env', '--project'].includes(a) && !['--broker', '--env', '--project'].includes(all[i - 1]));
	const r = spawnSync(process.execPath, [
		path.join(__dirname, 'launch.js'),
		'--launch-link', uri,
		'--settings', JSON.stringify({ 'opentunnel.brokerUrl': broker }),
		...extra,
	], { stdio: 'inherit' });
	process.exit(r.status ?? 1);
})();
