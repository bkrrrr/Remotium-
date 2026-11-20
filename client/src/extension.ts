import * as vscode from 'vscode';
import { HostStore } from './hosts';
import { AUTHORITY_PREFIX, hostIdFromAuthority, remoteUri, RemotiumResolver } from './resolver';
import { setUp } from './setup';
import { Pairing } from './uriHandler';

export function activate(context: vscode.ExtensionContext): void {
	const log = vscode.window.createOutputChannel('Remotium', { log: true });
	context.subscriptions.push(log);

	const hosts = new HostStore(context, log);
	const resolver = new RemotiumResolver(hosts, log);
	// Without "enable-proposed-api" in argv.json (a fresh install), using the resolver API
	// throws. Everything else still works, so the extension can offer to fix it.
	let apiAvailable = true;
	try {
		context.subscriptions.push(vscode.workspace.registerRemoteAuthorityResolver(AUTHORITY_PREFIX, resolver));
	} catch (e) {
		apiAvailable = false;
		log.warn(`remote connection API not available: ${(e as Error).message.split('\n')[0]}`);
		if (!vscode.env.remoteName) {
			void setUp(log);
		}
	}
	const pairing = new Pairing(hosts, log, () => (apiAvailable ? true : setUp(log, { modal: true }).then(() => false)));
	context.subscriptions.push(vscode.window.registerUriHandler(pairing), { dispose: () => resolver.status?.dispose() });

	/** The host of this window, if it is a Remotium window. env.remoteAuthority is proposed API. */
	const currentHostId = (): string | undefined => {
		const folder = vscode.workspace.workspaceFolders?.[0]?.uri;
		return vscode.env.remoteName === AUTHORITY_PREFIX && folder?.scheme === 'vscode-remote' ? hostIdFromAuthority(folder.authority) : undefined;
	};

	const pickHost = async (placeHolder: string) => {
		const list = hosts.list();
		if (!list.length) {
			void vscode.window.showInformationMessage('Remotium: no hosts yet. Run `remotium-host pair` on the machine and open the link, or use "Remotium: Add Host…".');
			return undefined;
		}
		const pick = await vscode.window.showQuickPick(
			list.map(h => ({ label: h.name, description: h.url, detail: h.folder, host: h })),
			{ placeHolder },
		);
		return pick?.host;
	};

	context.subscriptions.push(
		vscode.commands.registerCommand('remotium.addHost', async (link?: string, opts?: { confirm?: boolean; open?: boolean }) => {
			link ??= await vscode.window.showInputBox({ prompt: 'Paste the pairing link printed by `remotium-host pair`', ignoreFocusOut: true });
			return link ? (await pairing.pairWith(link, opts)) !== undefined : false;
		}),
		vscode.commands.registerCommand('remotium.openHost', async () => {
			const h = await pickHost('Open which host?');
			if (h) {
				await vscode.commands.executeCommand('vscode.openFolder', remoteUri(h, h.folder), { forceNewWindow: true });
			}
		}),
		vscode.commands.registerCommand('remotium.removeHost', async () => {
			const h = await pickHost('Forget which host? (Revoke it on the host with `remotium-host revoke`.)');
			if (h) {
				await hosts.remove(h.id);
				void vscode.window.showInformationMessage(`Remotium: forgot ${h.name}. On the host, run \`remotium-host revoke ${h.deviceId}\` to revoke this VSCodium.`);
			}
		}),
		vscode.commands.registerCommand('remotium.disconnect', async () => {
			if (currentHostId()) {
				await vscode.commands.executeCommand('workbench.action.remote.close');
			}
		}),
		vscode.commands.registerCommand('remotium.showLog', () => log.show()),
		vscode.commands.registerCommand('remotium.setUp', (opts?: { confirm?: boolean }) => setUp(log, { confirm: opts?.confirm ?? true, modal: true })),
		vscode.commands.registerCommand('remotium.menu', async () => {
			const id = currentHostId();
			const h = id ? hosts.get(id) : undefined;
			const pick = await vscode.window.showQuickPick(
				[
					{ label: '$(debug-disconnect) Disconnect', id: 'remotium.disconnect' },
					{ label: '$(remote) Open Host…', id: 'remotium.openHost' },
					{ label: '$(output) Show Log', id: 'remotium.showLog' },
				],
				{ placeHolder: h ? `Remotium: ${h.name} (${h.url})` : 'Remotium' },
			);
			if (pick) {
				await vscode.commands.executeCommand(pick.id);
			}
		}),
	);
	log.info(`Remotium activated (remote: ${vscode.env.remoteName ?? 'none'}, API ${apiAvailable ? 'available' : 'not enabled'})`);
}

export function deactivate(): void {}
