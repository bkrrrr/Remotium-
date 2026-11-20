// Runs inside the local extension host of a remote window opened on an Remotium authority.
// Checks that the remote file system and terminals work, then writes a JSON result file.
const vscode = require('vscode');
const fs = require('fs');

async function waitFor(pred, ms, what) {
	const end = Date.now() + ms;
	while (Date.now() < end) {
		if (await pred()) return;
		await new Promise(r => setTimeout(r, 250));
	}
	throw new Error(`timed out waiting for ${what}`);
}

exports.run = async function run() {
	const result = { ok: false, steps: [] };
	const step = s => { result.steps.push(s); console.log('[smoke]', s); };
	try {
		const folder = vscode.workspace.workspaceFolders?.[0];
		if (!folder) throw new Error('no workspace folder');
		step(`folder ${folder.uri.toString()} remoteName=${vscode.env.remoteName}`);
		if (folder.uri.scheme !== 'vscode-remote') throw new Error('workspace is not remote');
		// The test can start before the remote connection is resolved.
		const t0 = Date.now();
		await waitFor(async () => {
			try { await vscode.workspace.fs.stat(folder.uri); return true; } catch { return false; }
		}, 120000, 'remote file system');
		step(`remote file system ready after ${Date.now() - t0} ms`);

		const readme = vscode.Uri.joinPath(folder.uri, 'README.md');
		const marker = `edited over remotium ${Date.now()}`;
		await vscode.workspace.fs.writeFile(readme, Buffer.from(`# demo\n\n${marker}\n`));
		const back = Buffer.from(await vscode.workspace.fs.readFile(readme)).toString();
		if (!back.includes(marker)) throw new Error('read-back mismatch');
		step('fs write/read ok');
		result.marker = marker;

		const entries = await vscode.workspace.fs.readDirectory(folder.uri);
		step(`readDirectory: ${entries.map(e => e[0]).join(',')}`);

		const term = vscode.window.createTerminal({ name: 'smoke' });
		const pidOk = await Promise.race([term.processId.then(p => p !== undefined), new Promise(r => setTimeout(() => r(false), 30000))]);
		if (!pidOk) throw new Error('terminal did not start');
		const termOut = vscode.Uri.joinPath(folder.uri, 'terminal-ok.txt');
		term.sendText(`echo terminal-ok > '${termOut.path}'`);
		await waitFor(async () => {
			try { await vscode.workspace.fs.stat(termOut); return true; } catch { return false; }
		}, 30000, 'terminal output file');
		await vscode.workspace.fs.delete(termOut);
		term.dispose();
		step('terminal ok');

		const remoteExt = vscode.extensions.all.filter(e => e.extensionKind === vscode.ExtensionKind.Workspace).map(e => e.id);
		step(`remote extensions: ${remoteExt.length}`);
		result.ok = true;
	} catch (e) {
		result.error = String(e && e.stack || e);
		console.error('[smoke] FAILED', e);
	} finally {
		if (process.env.SMOKE_RESULT) fs.writeFileSync(process.env.SMOKE_RESULT, JSON.stringify(result, null, 2));
	}
	if (!result.ok) throw new Error(result.error);
};
