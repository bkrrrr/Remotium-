// Drives the real user flow for the server e2e without the test runner (which would make
// SecretStorage in-memory and per window):
//  - local window: open the launch link like the URI handler would (sign in, open remote window)
//  - remote window: run smoke.js, write the result file, quit.
const vscode = require('vscode');
const fs = require('fs');
const path = require('path');

exports.activate = async function activate() {
	const cfg = vscode.workspace.getConfiguration('remotiumE2e');
	const resultFile = cfg.get('resultFile');
	if (!resultFile) return;
	const write = r => fs.writeFileSync(resultFile, JSON.stringify(r, null, 2));
	try {
		if (vscode.env.remoteName === 'remotium') {
			process.env.SMOKE_RESULT = resultFile;
			const suite = cfg.get('suite') || 'smoke';
			await require(path.join(__dirname, '..', `${suite}.js`)).run().catch(() => {});
		} else {
			const ok = await vscode.commands.executeCommand('remotium.openLaunchLink', cfg.get('launchLink'));
			if (ok) return; // The remote window takes over.
			write({ ok: false, error: 'opening the launch link failed (see the Remotium log)' });
		}
	} catch (e) {
		write({ ok: false, error: String(e && e.stack || e) });
	}
	await vscode.commands.executeCommand('workbench.action.quit');
};
