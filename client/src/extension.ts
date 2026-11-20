import * as vscode from 'vscode';
import { AuthStore } from './auth';
import { AUTHORITY_PREFIX, DIRECT_ENV, envFromAuthority, RemotiumResolver } from './resolver';
import { LaunchUriHandler } from './uriHandler';

export function activate(context: vscode.ExtensionContext): void {
	const log = vscode.window.createOutputChannel('Remotium', { log: true });
	context.subscriptions.push(log);

	const auth = new AuthStore(context, log);
	const resolver = new RemotiumResolver(auth, log);
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
		vscode.commands.registerCommand('remotium.connectDirect', async () => {
			const folder = await vscode.window.showInputBox({ prompt: 'Remote folder to open', value: '/workspace/demo' });
			if (folder) {
				const uri = vscode.Uri.from({ scheme: 'vscode-remote', authority: `${AUTHORITY_PREFIX}+${DIRECT_ENV}`, path: folder });
				await vscode.commands.executeCommand('vscode.openFolder', uri, { forceNewWindow: true });
			}
		}),
		vscode.commands.registerCommand('remotium.disconnect', async () => {
			if (!remoteEnv()) {
				return;
			}
			await resolver.disconnect();
			await vscode.commands.executeCommand('workbench.action.remote.close');
		}),
		vscode.commands.registerCommand('remotium.signOut', async () => {
			const env = remoteEnv() ?? (await vscode.window.showInputBox({ prompt: 'Environment to sign out of' }));
			if (!env) {
				return;
			}
			await resolver.disconnect();
			await auth.signOut(env);
			void vscode.window.showInformationMessage(`Remotium: signed out of ${env}.`);
			if (remoteEnv()) {
				await vscode.commands.executeCommand('workbench.action.remote.close');
			}
		}),
		vscode.commands.registerCommand('remotium.showLog', () => log.show()),
		// For browsers that can't open vscodium:// links: paste the link instead.
		vscode.commands.registerCommand('remotium.openLaunchLink', async (link?: string, opts?: { openFolder?: boolean }) => {
			link ??= await vscode.window.showInputBox({ prompt: 'Paste the launch link from the Remotium dashboard', ignoreFocusOut: true });
			if (!link) {
				return false;
			}
			let uri: vscode.Uri;
			try {
				uri = vscode.Uri.parse(link.trim(), true);
			} catch {
				void vscode.window.showErrorMessage('Remotium: that is not a launch link.');
				return false;
			}
			return uriHandler.handle(uri, opts?.openFolder ?? true);
		}),
		vscode.commands.registerCommand('remotium.menu', async () => {
			const env = remoteEnv();
			const user = env ? await auth.user(env) : undefined;
			const pick = await vscode.window.showQuickPick(
				[
					{ label: '$(debug-disconnect) Disconnect', id: 'remotium.disconnect' },
					{ label: '$(sign-out) Sign Out', id: 'remotium.signOut' },
					{ label: '$(output) Show Log', id: 'remotium.showLog' },
				],
				{ placeHolder: env ? `Remotium: ${env}${user ? ` (${user})` : ''} via ${auth.serverFor(env) ?? '?'}` : 'Remotium' },
			);
			if (pick) {
				await vscode.commands.executeCommand(pick.id);
			}
		}),
	);
	log.info(`Remotium activated (remote: ${vscode.env.remoteAuthority ?? 'none'})`);
}

export function deactivate(): void {}
