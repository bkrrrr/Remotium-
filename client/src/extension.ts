import * as vscode from 'vscode';

export function activate(context: vscode.ExtensionContext): void {
	const log = vscode.window.createOutputChannel('OpenTunnel', { log: true });
	context.subscriptions.push(log);
	log.info('OpenTunnel activated');
}

export function deactivate(): void {}
