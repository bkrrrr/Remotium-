import * as vscode from 'vscode';
import { AuthStore } from './auth';
import { BrokerApi, BrokerError, SessionInfo } from './brokerApi';
import { getProductInfo } from './productInfo';
import { StatusItem } from './status';
import { connectWs, WsConnectError } from './wsMessagePassing';

export const AUTHORITY_PREFIX = 'opentunnel';
export const DIRECT_ENV = 'direct';

/** Close codes from the broker's data plane. */
const CLOSE = { unauthorized: 4401, forbidden: 4403, offline: 4404, versionUnavailable: 4409, internal: 4500 };

/** `opentunnel+<env>` → `<env>` */
export function envFromAuthority(authority: string): string {
	const plus = authority.indexOf('+');
	if (plus < 0 || authority.slice(0, plus) !== AUTHORITY_PREFIX) {
		throw new Error(`not an OpenTunnel authority: ${authority}`);
	}
	return decodeURIComponent(authority.slice(plus + 1));
}

interface ActiveSession extends SessionInfo {
	env: string;
	broker: string;
	streamTokenExpiresAt: number;
}

export class OpenTunnelResolver implements vscode.RemoteAuthorityResolver {
	private connectionCount = 0;
	private session: ActiveSession | undefined;
	status: StatusItem | undefined;

	constructor(private readonly auth: AuthStore, private readonly log: vscode.LogOutputChannel) {}

	get currentSession(): ActiveSession | undefined {
		return this.session;
	}

	async resolve(authority: string, context: vscode.RemoteAuthorityResolverContext): Promise<vscode.ResolverResult> {
		const env = envFromAuthority(authority);
		this.log.info(`resolving ${authority} (attempt ${context.resolveAttempt})`);
		if (env === DIRECT_ENV) {
			return this.resolveDirect();
		}
		this.status ??= new StatusItem(env);
		this.status.set('resolving');
		try {
			const s = await this.createSession(env);
			this.log.info(`session ${s.sessionId} on ${s.broker}`);
			return new vscode.ManagedResolvedAuthority(() => this.connect(env), s.connectionToken);
		} catch (e) {
			this.status.set('error', (e as Error).message);
			throw toResolverError(e);
		}
	}

	private async createSession(env: string): Promise<ActiveSession> {
		const product = getProductInfo();
		const version = { commit: product.commit, quality: product.quality, version: product.version, release: product.release };
		const attempt = async (force: boolean) => {
			const { broker, token } = await this.auth.accessToken(env, force);
			const info = await new BrokerApi(broker).createSession(token, env, version);
			return { ...info, env, broker, streamTokenExpiresAt: Date.now() + info.streamTokenExpiresIn * 1000 };
		};
		let s: ActiveSession;
		try {
			s = await attempt(false);
		} catch (e) {
			if (!(e instanceof BrokerError) || e.status !== 401 || e.code === 'signed_out') {
				throw e;
			}
			s = await attempt(true);
		}
		this.session = s;
		return s;
	}

	/** A stream token that is still valid for a few seconds, renewing the session if needed. */
	private async streamCredentials(env: string, forceNew: boolean): Promise<ActiveSession> {
		let s = this.session;
		if (!s || s.env !== env) {
			return this.createSession(env);
		}
		if (!forceNew && s.streamTokenExpiresAt - Date.now() > 10_000) {
			return s;
		}
		try {
			const { broker, token } = await this.auth.accessToken(env);
			const t = await new BrokerApi(broker).streamToken(token, s.sessionId);
			s = { ...s, streamToken: t.streamToken, streamTokenExpiresAt: Date.now() + t.expiresIn * 1000 };
			this.session = s;
			return s;
		} catch (e) {
			// The broker forgot the session (e.g. it restarted): start a new one.
			if (e instanceof BrokerError && (e.status === 404 || e.status === 401) && e.code !== 'signed_out') {
				this.log.info(`session ${s.sessionId} is gone; creating a new one`);
				return this.createSession(env);
			}
			throw e;
		}
	}

	/** Called by the editor for every connection, including reconnects. */
	private async connect(env: string): Promise<vscode.ManagedMessagePassing> {
		const n = ++this.connectionCount;
		const label = `${env}#${n}`;
		for (let attempt = 0; ; attempt++) {
			const s = await this.streamCredentials(env, attempt > 0);
			try {
				const mp = await connectWs(s.streamUrl, {
					headers: { authorization: `Bearer ${s.streamToken}` },
					log: this.log,
					label,
					onClosed: err => this.status?.closed(err),
				});
				this.status?.opened();
				return mp;
			} catch (e) {
				const code = e instanceof WsConnectError ? e.closeCode : undefined;
				this.log.warn(`${label}: connect failed: ${(e as Error).message}`);
				if (code === CLOSE.unauthorized && attempt === 0) {
					continue;
				}
				this.status?.set('reconnecting', (e as Error).message);
				throw e;
			}
		}
	}

	async disconnect(): Promise<void> {
		const s = this.session;
		if (!s) {
			return;
		}
		this.session = undefined;
		try {
			const { broker, token } = await this.auth.accessToken(s.env);
			await new BrokerApi(broker).revokeSession(token, s.sessionId);
		} catch (e) {
			this.log.warn(`revoking session failed: ${(e as Error).message}`);
		}
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

function toResolverError(e: unknown): Error {
	if (e instanceof BrokerError) {
		switch (e.code) {
			case 'network':
			case 'offline':
				return vscode.RemoteAuthorityResolverError.TemporarilyNotAvailable(e.message);
			case 'version_unavailable':
				return vscode.RemoteAuthorityResolverError.NotAvailable(`No matching server for this VSCodium version: ${e.message}`, false);
		}
		if (e.isAuthFailure) {
			return vscode.RemoteAuthorityResolverError.NotAvailable(`${e.message} Open the environment again from the OpenTunnel dashboard.`, false);
		}
		return vscode.RemoteAuthorityResolverError.NotAvailable(e.message, false);
	}
	return e instanceof Error ? e : new Error(String(e));
}
