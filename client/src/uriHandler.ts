import * as os from 'os';
import * as vscode from 'vscode';
import { HostError, pair } from './hostApi';
import { HostRecord, HostStore } from './hosts';
import { parsePairingLink } from './pairing';
import { remoteUri } from './resolver';

export interface PairOptions {
	/** Ask before pairing. `false` is for scripted setups and tests. */
	confirm?: boolean;
	/** Open the host's folder afterwards (default true). */
	open?: boolean;
}

/** Pairs with a host from a pairing link and opens its folder. */
export class Pairing implements vscode.UriHandler {
	constructor(
		private readonly hosts: HostStore,
		private readonly log: vscode.LogOutputChannel,
		/** Resolves to false (after offering setup) when the remote API isn't enabled yet. */
		private readonly ready: () => boolean | Promise<boolean>,
	) {}

	async handleUri(uri: vscode.Uri): Promise<void> {
		if (uri.path === '/pair') {
			await this.pairWith(uri.toString(true));
		} else {
			this.log.warn(`ignoring unknown URI path ${uri.path}`);
		}
	}

	async pairWith(link: string, opts: PairOptions = {}): Promise<HostRecord | undefined> {
		if (!(await this.ready())) {
			this.log.warn('pairing postponed: finish the setup and restart VSCodium first');
			return undefined;
		}
		let parsed;
		try {
			parsed = parsePairingLink(link);
		} catch (e) {
			void vscode.window.showErrorMessage(`Remotium: ${(e as Error).message}`);
			return undefined;
		}
		if (opts.confirm !== false) {
			const detail = parsed.fingerprint
				? `Certificate SHA-256: ${parsed.fingerprint.match(/../g)!.join(':').toUpperCase()}\n\nOnly continue if you created this link with \`remotium-host pair\` on that machine.`
				: 'Only continue if you created this link with `remotium-host pair` on that machine.';
			const choice = await vscode.window.showWarningMessage(`Pair with the Remotium host at ${parsed.url}?`, { modal: true, detail }, 'Pair');
			if (choice !== 'Pair') {
				return undefined;
			}
		}
		let host: HostRecord;
		try {
			const r = await vscode.window.withProgress(
				{ location: vscode.ProgressLocation.Notification, title: `Remotium: pairing with ${parsed.url}…` },
				() => pair(parsed, parsed.code, `VSCodium on ${os.hostname()}`),
			);
			host = { id: r.hostId, url: parsed.url, name: r.name, folder: r.folder, fingerprint: parsed.fingerprint, deviceId: r.deviceId };
			await this.hosts.add(host, r.deviceToken);
		} catch (e) {
			const msg = (e as Error).message;
			this.log.error(`pairing failed: ${msg}`);
			const hint = e instanceof HostError && e.code === 'certificate' ? '' : ' Pairing links work once and expire after 10 minutes; create a new one with `remotium-host pair`.';
			void vscode.window.showErrorMessage(`Remotium: pairing failed: ${msg}.${hint}`);
			return undefined;
		}
		this.log.info(`paired with ${host.name} (${host.url}) as device ${host.deviceId}`);
		if (opts.open !== false) {
			await vscode.commands.executeCommand('vscode.openFolder', remoteUri(host, host.folder), { forceNewWindow: true });
		}
		return host;
	}
}
