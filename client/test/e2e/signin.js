// Phase 1 of the broker e2e: sign in with a launch link (no folder is opened).
const vscode = require('vscode');
const fs = require('fs');

exports.run = async function run() {
	const result = { ok: false, steps: [] };
	try {
		const ext = vscode.extensions.getExtension('opentunnel.client');
		await ext.activate();
		const ok = await vscode.commands.executeCommand('opentunnel.openLaunchLink', process.env.OPENTUNNEL_LAUNCH_LINK, { openFolder: false });
		if (!ok) throw new Error('sign-in returned false');
		result.steps.push('signed in');
		result.ok = true;
	} catch (e) {
		result.error = String(e && e.stack || e);
	} finally {
		if (process.env.SMOKE_RESULT) fs.writeFileSync(process.env.SMOKE_RESULT, JSON.stringify(result, null, 2));
	}
	if (!result.ok) throw new Error(result.error);
};
