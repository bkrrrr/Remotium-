import * as vscode from 'vscode';
import { connectWs } from './wsMessagePassing';

export const AUTHORITY_PREFIX = 'opentunnel';
export const DIRECT_ENV = 'direct';

/** `opentunnel+<env>` → `<env>` */
export function envFromAuthority(authority: string): string {
	const plus = authority.indexOf('+');
	if (plus < 0 || authority.slice(0, plus) !== AUTHORITY_PREFIX) {
		throw new Error(`not an OpenTunnel authority: ${authority}`);
	}
	return decodeURIComponent(authority.slice(plus + 1));
}

export class OpenTunnelResolver implements vscode.RemoteAuthorityResolver {
	private connectionCount = 0;

	constructor(private readonly log: vscode.LogOutputChannel) {}

	async resolve(authority: string, context: vscode.RemoteAuthorityResolverContext): Promise<vscode.ResolverResult> {
		const env = envFromAuthority(authority);
		this.log.info(`resolving ${authority} (attempt ${context.resolveAttempt})`);
		if (env === DIRECT_ENV) {
			return this.resolveDirect();
		}
		throw vscode.RemoteAuthorityResolverError.NotAvailable(`Broker mode is not implemented yet (${env})`, true);
	}

	/** M0 spike: connect straight to an agent in --direct-listen mode. */
	private resolveDirect(): vscode.ResolverResult {
		const cfg = vscode.workspace.getConfiguration('opentunnel');
		const url = cfg.get<string>('directUrl');
		if (!url) {
			throw vscode.RemoteAuthorityResolverError.NotAvailable('Set opentunnel.directUrl to use the direct connection.', false);
		}
		const token = cfg.get<string>('directConnectionToken') || undefined;
		return new vscode.ManagedResolvedAuthority(() => {
			const n = ++this.connectionCount;
			return connectWs(url, { log: this.log, label: `direct#${n}` });
		}, token);
	}
}
