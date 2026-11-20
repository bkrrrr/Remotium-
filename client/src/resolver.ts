import * as vscode from 'vscode';
import { agentFor, createSession, HostError } from './hostApi';
import { HostRecord, HostStore } from './hosts';
import { toWebSocketUrl } from './pairing';
import { getProductInfo } from './productInfo';
import { StatusItem } from './status';
import { connectWs, WsConnectError } from './wsMessagePassing';

export const AUTHORITY_PREFIX = 'remotium';

/** `remotium+<hostId>` → `<hostId>` */
export function hostIdFromAuthority(authority: string): string {
	const plus = authority.indexOf('+');
	if (plus < 0 || authority.slice(0, plus) !== AUTHORITY_PREFIX) {
		throw new Error(`not a Remotium authority: ${authority}`);
	}
	return decodeURIComponent(authority.slice(plus + 1));
}

export function remoteUri(host: Pick<HostRecord, 'id'>, folder: string): vscode.Uri {
	return vscode.Uri.from({ scheme: 'vscode-remote', authority: `${AUTHORITY_PREFIX}+${host.id}`, path: folder });
}

export class RemotiumResolver implements vscode.RemoteAuthorityResolver {
	private connectionCount = 0;
	status: StatusItem | undefined;

	constructor(private readonly hosts: HostStore, private readonly log: vscode.LogOutputChannel) {}

	async resolve(authority: string, context: vscode.RemoteAuthorityResolverContext): Promise<vscode.ResolverResult> {
		const id = hostIdFromAuthority(authority);
		this.log.info(`resolving ${authority} (attempt ${context.resolveAttempt})`);
		const loaded = await this.hosts.load(id);
		if (!loaded) {
			throw vscode.RemoteAuthorityResolverError.NotAvailable(`This VSCodium isn't paired with host ${id}. Run \`remotium-host pair\` on the host and open the link.`, false);
		}
		const { host, token } = loaded;
		this.status ??= new StatusItem(host.name);
		this.status.set('resolving');
		try {
			const p = getProductInfo();
			const { connectionToken } = await createSession(host, token, { commit: p.commit, quality: p.quality, version: p.version, release: p.release });
			const streamUrl = `${toWebSocketUrl(host.url)}/remotium/v1/stream?commit=${encodeURIComponent(p.commit)}`;
			this.log.info(`session on ${host.url}`);
			return new vscode.ManagedResolvedAuthority(() => this.connect(host, token, streamUrl), connectionToken);
		} catch (e) {
			this.status.set('error', (e as Error).message);
			throw toResolverError(e);
		}
	}

	/** Called by the editor for every connection, including reconnects. */
	private async connect(host: HostRecord, token: string, url: string): Promise<vscode.ManagedMessagePassing> {
		const label = `${host.name}#${++this.connectionCount}`;
		this.log.info(`${label}: connecting`);
		try {
			const mp = await connectWs(url, {
				headers: { authorization: `Bearer ${token}` },
				agent: agentFor(host),
				log: this.log,
				label,
				onClosed: err => this.status?.closed(err),
			});
			this.status?.opened();
			this.log.info(`${label}: connected`);
			return mp;
		} catch (e) {
			this.log.warn(`${label}: connect failed: ${(e as Error).message}`);
			this.status?.set('reconnecting', (e as Error).message);
			if (e instanceof WsConnectError && e.status === 401) {
				throw vscode.RemoteAuthorityResolverError.NotAvailable(`Host ${host.name} no longer accepts this VSCodium (device revoked?). Pair again.`, false);
			}
			throw e;
		}
	}
}

function toResolverError(e: unknown): Error {
	if (e instanceof HostError) {
		if (e.code === 'network') {
			return vscode.RemoteAuthorityResolverError.TemporarilyNotAvailable(e.message);
		}
		if (e.code === 'version_unavailable') {
			return vscode.RemoteAuthorityResolverError.NotAvailable(`No VSCodium server for this version on the host: ${e.message}`, false);
		}
		if (e.isAuthFailure) {
			return vscode.RemoteAuthorityResolverError.NotAvailable(`${e.message} Run \`remotium-host pair\` on the host and open the new link.`, false);
		}
		return vscode.RemoteAuthorityResolverError.NotAvailable(e.message, false);
	}
	return e instanceof Error ? e : new Error(String(e));
}
