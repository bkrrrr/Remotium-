import * as vscode from 'vscode';
import { AUTHORITY_PREFIX, DIRECT_ENV, OpenTunnelResolver } from './resolver';

export function activate(context: vscode.ExtensionContext): void {
	const log = vscode.window.createOutputChannel('OpenTunnel', { log: true });
	context.subscriptions.push(log);

	const resolver = new OpenTunnelResolver(log);
	context.subscriptions.push(vscode.workspace.registerRemoteAuthorityResolver(AUTHORITY_PREFIX, resolver));

	context.subscriptions.push(
		vscode.commands.registerCommand('opentunnel.connectDirect', async () => {
			const folder = await vscode.window.showInputBox({ prompt: 'Remote folder to open', value: '/workspace/demo' });
			if (folder) {
				const uri = vscode.Uri.from({ scheme: 'vscode-remote', authority: `${AUTHORITY_PREFIX}+${DIRECT_ENV}`, path: folder });
				await vscode.commands.executeCommand('vscode.openFolder', uri, { forceNewWindow: true });
			}
		}),
		vscode.commands.registerCommand('opentunnel.showLog', () => log.show()),
	);
	log.info(`OpenTunnel activated (remote: ${vscode.env.remoteName ?? 'none'})`);
}

export function deactivate(): void {}
