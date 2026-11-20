// Phase 3 validation, run in the remote window: terminals, port forwarding, Node and Python
// debugging, installing an Open VSX extension remotely, reconnecting after a server restart
// and after an agent restart. Restarts are done by the outer runner (run-server.js), which
// watches <controlDir>/request-<name> and answers with <controlDir>/done-<name>.
const vscode = require('vscode');
const fs = require('fs');
const path = require('path');

const sleep = ms => new Promise(r => setTimeout(r, ms));

async function waitFor(pred, ms, what) {
	const end = Date.now() + ms;
	let last;
	while (Date.now() < end) {
		try {
			// A call on a dead connection can hang; bound each attempt.
			const v = await Promise.race([pred(), sleep(10000).then(() => { throw new Error('attempt timed out'); })]);
			if (v) return v;
		} catch (e) { last = e; }
		await sleep(300);
	}
	throw new Error(`timed out waiting for ${what}${last ? ` (${last.message})` : ''}`);
}

async function request(name) {
	const dir = vscode.workspace.getConfiguration('remotiumE2e').get('controlDir');
	if (!dir) throw new Error('remotiumE2e.controlDir not set');
	fs.writeFileSync(path.join(dir, `request-${name}`), String(Date.now()));
	await waitFor(() => fs.existsSync(path.join(dir, `done-${name}`)), 120000, `runner to ${name}`);
}

const exists = async uri => { try { await vscode.workspace.fs.stat(uri); return true; } catch { return false; } };

async function terminalWrites(term, file, text) {
	term.sendText(`echo ${text} > '${file.path}'`);
	await waitFor(async () => (await exists(file)) && Buffer.from(await vscode.workspace.fs.readFile(file)).toString().includes(text), 30000, `terminal to write ${text}`);
}

async function hitsBreakpoint(folder, file, line, config, expr, expected) {
	vscode.debug.removeBreakpoints(vscode.debug.breakpoints);
	vscode.debug.addBreakpoints([new vscode.SourceBreakpoint(new vscode.Location(file, new vscode.Position(line, 0)))]);
	const stopped = new Promise(resolve => {
		const d = vscode.debug.onDidChangeActiveStackItem(item => {
			if (item && 'frameId' in item) { d.dispose(); resolve(item); }
		});
	});
	const started = await vscode.debug.startDebugging(folder, config);
	if (!started) throw new Error(`could not start ${config.type} debugging`);
	const frame = await Promise.race([stopped, sleep(90000).then(() => { throw new Error('breakpoint not hit'); })]);
	const r = await frame.session.customRequest('evaluate', { expression: expr, frameId: frame.frameId, context: 'repl' });
	await vscode.debug.stopDebugging(frame.session);
	vscode.debug.removeBreakpoints(vscode.debug.breakpoints);
	if (String(r.result) !== expected) throw new Error(`${expr} = ${r.result}, expected ${expected}`);
	return `breakpoint hit, ${expr} = ${r.result}`;
}

exports.run = async function run() {
	const result = { ok: false, steps: [], failed: [] };
	const only = (vscode.workspace.getConfiguration('remotiumE2e').get('only') || '').split(',').map(x => x.trim()).filter(Boolean);
	const step = async (name, fn) => {
		if (only.length && !only.some(o => name.startsWith(o)) && name !== 'remote ready' && name !== 'terminal') return;
		const t0 = Date.now();
		try {
			const detail = await fn();
			result.steps.push({ name, ok: true, ms: Date.now() - t0, detail });
		} catch (e) {
			result.steps.push({ name, ok: false, ms: Date.now() - t0, error: String(e && e.message || e) });
			result.failed.push(name);
		}
		console.log('[phase3]', JSON.stringify(result.steps[result.steps.length - 1]));
		if (process.env.SMOKE_RESULT) fs.writeFileSync(process.env.SMOKE_RESULT + '.partial', JSON.stringify(result, null, 2));
	};
	const folder = vscode.workspace.workspaceFolders[0];
	const root = folder.uri;
	const at = p => vscode.Uri.joinPath(root, p);

	await step('remote ready', () => waitFor(() => exists(root), 120000, 'remote file system'));

	let term;
	await step('terminal', async () => {
		term = vscode.window.createTerminal({ name: 'phase3', cwd: root });
		await terminalWrites(term, at('.phase3-term'), 'one');
		return `pid ${await term.processId}`;
	});

	await step('port forwarding', async () => {
		const server = vscode.window.createTerminal({ name: 'http.server', cwd: root });
		server.sendText('python3 -m http.server 8000 --bind 127.0.0.1');
		const local = await vscode.env.asExternalUri(vscode.Uri.parse('http://127.0.0.1:8000/README.md'));
		const text = await waitFor(async () => {
			const res = await fetch(local.toString(true), { signal: AbortSignal.timeout(5000) });
			return res.ok && (await res.text());
		}, 60000, `GET ${local}`);
		const expected = Buffer.from(await vscode.workspace.fs.readFile(at('README.md'))).toString();
		if (text !== expected) throw new Error('forwarded response differs from the remote README.md');
		server.dispose();
		return `remote 8000 → ${local.authority}`;
	});

	await step('debug node', async () => {
		const file = at('dbg.js');
		await vscode.workspace.fs.writeFile(file, Buffer.from('let x = 41;\nx++;\nconsole.log(x);\n'));
		return hitsBreakpoint(folder, file, 1, { type: 'node', request: 'launch', name: 'dbg node', program: file.path }, 'x', '41');
	});

	await step('install Open VSX extension remotely (ms-python.debugpy)', async () => {
		// Resolves once installed. Remote extensions aren't listed in this (local) extension
		// host's vscode.extensions, so the Python debugging step below is the proof it runs.
		await vscode.commands.executeCommand('workbench.extensions.installExtension', 'ms-python.debugpy');
		return 'installed';
	});

	await step('debug python', async () => {
		const file = at('dbg.py');
		await vscode.workspace.fs.writeFile(file, Buffer.from('x = 41\nx += 1\nprint(x)\n'));
		return hitsBreakpoint(folder, file, 1, { type: 'debugpy', request: 'launch', name: 'dbg py', program: file.path, console: 'internalConsole', justMyCode: true }, 'x', '41');
	});

	const reconnects = async name => {
		await request(name);
		await waitFor(async () => {
			await vscode.workspace.fs.writeFile(at('.phase3-reconnect'), Buffer.from(name));
			return Buffer.from(await vscode.workspace.fs.readFile(at('.phase3-reconnect'))).toString() === name;
		}, 120000, 'file system after restart');
		// Same terminal process: the session survived, nothing was reloaded.
		await terminalWrites(term, at('.phase3-term'), name);
		return 'file system and existing terminal work, no reload';
	};
	await step('reconnect after server restart', () => reconnects('restart-server'));
	await step('reconnect after agent restart', () => reconnects('restart-agent'));

	for (const f of ['.phase3-term', '.phase3-reconnect', 'dbg.js', 'dbg.py']) {
		try { await vscode.workspace.fs.delete(at(f)); } catch { }
	}
	result.ok = result.failed.length === 0;
	if (process.env.SMOKE_RESULT) fs.writeFileSync(process.env.SMOKE_RESULT, JSON.stringify(result, null, 2));
};
