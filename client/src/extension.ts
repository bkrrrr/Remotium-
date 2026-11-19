import * as vscode from 'vscode';
import { AuthStore } from './auth';
import { AUTHORITY_PREFIX, DIRECT_ENV, envFromAuthority, OpenTunnelResolver } from './resolver';
import { LaunchUriHandler } from './uriHandler';

export function activate(context: vscode.ExtensionContext): void {
	const log = vscode.window.createOutputChannel('OpenTunnel', { log: true });
	context.subscriptions.push(log);

	const auth = new AuthStore(context, log);
	const resolver = new OpenTunnelResolver(auth, log);
	const uriHandler = new LaunchUriHandler(auth, log);
	context.subscriptions.push(
		vscode.workspace.registerRemoteAuthorityResolver(AUTHORITY_PREFIX, resolver),
		vscode.window.registerUriHandler(uriHandler),
		{ dispose: () => resolver.status?.dispose() },
	);

	const remoteEnv = (): string | undefined => {
		const a = vscode.env.remoteAuthority;
		return a?.startsWith(`${AUTHORITY_PREFIX}+`) ? envFromAuthority(a) : undefined;
	};

	context.subscriptions.push(
		vscode.commands.registerCommand('opentunnel.connectDirect', async () => {
			const folder = await vscode.window.showInputBox({ prompt: 'Remote folder to open', value: '/workspace/demo' });
			if (folder) {
				const uri = vscode.Uri.from({ scheme: 'vscode-remote', authority: `${AUTHORITY_PREFIX}+${DIRECT_ENV}`, path: folder });
				await vscode.commands.executeCommand('vscode.openFolder', uri, { forceNewWindow: true });
			}
		}),
		vscode.commands.registerCommand('opentunnel.disconnect', async () => {
			if (!remoteEnv()) {
				return;
			}
			await resolver.disconnect();
			await vscode.commands.executeCommand('workbench.action.remote.close');
		}),
		vscode.commands.registerCommand('opentunnel.signOut', async () => {
			const env = remoteEnv() ?? (await vscode.window.showInputBox({ prompt: 'Environment to sign out of' }));
			if (!env) {
				return;
			}
			await resolver.disconnect();
			await auth.signOut(env);
			void vscode.window.showInformationMessage(`OpenTunnel: signed out of ${env}.`);
			if (remoteEnv()) {
				await vscode.commands.executeCommand('workbench.action.remote.close');
			}
		}),
		vscode.commands.registerCommand('opentunnel.showLog', () => log.show()),
		// For browsers that can't open vscodium:// links: paste the link instead.
		vscode.commands.registerCommand('opentunnel.openLaunchLink', async (link?: string, opts?: { openFolder?: boolean }) => {
			link ??= await vscode.window.showInputBox({ prompt: 'Paste the launch link from the OpenTunnel dashboard', ignoreFocusOut: true });
			if (!link) {
				return false;
			}
			let uri: vscode.Uri;
			try {
				uri = vscode.Uri.parse(link.trim(), true);
			} catch {
				void vscode.window.showErrorMessage('OpenTunnel: that is not a launch link.');
				return false;
			}
			return uriHandler.handle(uri, opts?.openFolder ?? true);
		}),
		vscode.commands.registerCommand('opentunnel.menu', async () => {
			const env = remoteEnv();
			const user = env ? await auth.user(env) : undefined;
			const pick = await vscode.window.showQuickPick(
				[
					{ label: '$(debug-disconnect) Disconnect', id: 'opentunnel.disconnect' },
					{ label: '$(sign-out) Sign Out', id: 'opentunnel.signOut' },
					{ label: '$(output) Show Log', id: 'opentunnel.showLog' },
				],
				{ placeHolder: env ? `OpenTunnel: ${env}${user ? ` (${user})` : ''} via ${auth.brokerFor(env) ?? '?'}` : 'OpenTunnel' },
			);
			if (pick) {
				await vscode.commands.executeCommand(pick.id);
			}
		}),
	);
	log.info(`OpenTunnel activated (remote: ${vscode.env.remoteAuthority ?? 'none'})`);
}

export function deactivate(): void {}
