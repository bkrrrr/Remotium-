import * as vscode from 'vscode';
import { AuthStore } from './auth';
import { AUTHORITY_PREFIX } from './resolver';

/**
 * Handles `<scheme>://opentunnel.client/connect?ticket=…&env=…&path=…&broker=…` from the
 * dashboard: trades the one-time ticket for tokens and opens the remote folder.
 */
export class LaunchUriHandler implements vscode.UriHandler {
	constructor(private readonly auth: AuthStore, private readonly log: vscode.LogOutputChannel) {}

	async handleUri(uri: vscode.Uri): Promise<void> {
		await this.handle(uri, true);
	}

	/** `openFolder = false` only signs in (used by tests and scripted setups). */
	async handle(uri: vscode.Uri, openFolder: boolean): Promise<boolean> {
		if (uri.path !== '/connect') {
			this.log.warn(`ignoring unknown URI path ${uri.path}`);
			return false;
		}
		const q = new URLSearchParams(uri.query);
		const ticket = q.get('ticket');
		const env = q.get('env');
		const path = q.get('path') || '/';
		const broker = q.get('broker') || vscode.workspace.getConfiguration('opentunnel').get<string>('brokerUrl');
		if (!ticket || !env || !broker) {
			void vscode.window.showErrorMessage('OpenTunnel: the launch link is incomplete.');
			return false;
		}
		if (!/^https?:\/\//.test(broker) || !/^[A-Za-z0-9_-]{1,64}$/.test(env) || !path.startsWith('/')) {
			void vscode.window.showErrorMessage('OpenTunnel: the launch link is malformed.');
			return false;
		}
		if (!this.auth.isTrustedBroker(broker)) {
			const insecure = broker.startsWith('http://') && !/^http:\/\/(localhost|127\.0\.0\.1)(:|\/|$)/.test(broker);
			const choice = await vscode.window.showWarningMessage(
				`Connect to the OpenTunnel broker at ${broker}?`,
				{
					modal: true,
					detail: `The broker will get a remote window into environment "${env}". Only continue if you opened this link from your own OpenTunnel dashboard.${insecure ? '\n\nThis broker uses plain HTTP, so traffic is not encrypted.' : ''}`,
				},
				'Connect',
			);
			if (choice !== 'Connect') {
				return false;
			}
			await this.auth.trustBroker(broker);
		}
		try {
			await vscode.window.withProgress(
				{ location: vscode.ProgressLocation.Notification, title: `OpenTunnel: signing in to ${env}…` },
				() => this.auth.signIn(broker, ticket),
			);
		} catch (e) {
			this.log.error(`sign-in failed: ${(e as Error).message}`);
			void vscode.window.showErrorMessage(`OpenTunnel: sign-in failed: ${(e as Error).message}. Launch links work once and expire after 60 seconds; open it again from the dashboard.`);
			return false;
		}
		if (!openFolder) {
			return true;
		}
		const target = vscode.Uri.from({ scheme: 'vscode-remote', authority: `${AUTHORITY_PREFIX}+${env}`, path });
		this.log.info(`opening ${target.toString()}`);
		await vscode.commands.executeCommand('vscode.openFolder', target, { forceNewWindow: true });
		return true;
	}
}
