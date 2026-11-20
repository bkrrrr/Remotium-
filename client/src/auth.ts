import * as vscode from 'vscode';
import { ServerApi, ServerError, TokenPair } from './serverApi';

interface StoredTokens {
	server: string;
	user: string;
	accessToken: string;
	accessExpiresAt: number;
	refreshToken: string;
	refreshExpiresAt: number;
}

const ENV_SERVERS_KEY = 'remotium.envServers';
const TRUSTED_SERVERS_KEY = 'remotium.trustedServers';
const secretKey = (env: string) => `remotium.tokens.${env}`;

/** Per-environment client tokens in SecretStorage, refreshed on demand. */
export class AuthStore {
	private refreshing = new Map<string, Promise<StoredTokens>>();

	constructor(private readonly ctx: vscode.ExtensionContext, private readonly log: vscode.LogOutputChannel) {}

	serverFor(env: string): string | undefined {
		const map = this.ctx.globalState.get<Record<string, string>>(ENV_SERVERS_KEY, {});
		return map[env] || vscode.workspace.getConfiguration('remotium').get<string>('serverUrl') || undefined;
	}

	isTrustedServer(url: string): boolean {
		const configured = vscode.workspace.getConfiguration('remotium').get<string>('serverUrl');
		const trusted = this.ctx.globalState.get<string[]>(TRUSTED_SERVERS_KEY, []);
		return normalize(url) === normalize(configured ?? '') || trusted.includes(normalize(url));
	}

	async trustServer(url: string): Promise<void> {
		const trusted = new Set(this.ctx.globalState.get<string[]>(TRUSTED_SERVERS_KEY, []));
		trusted.add(normalize(url));
		await this.ctx.globalState.update(TRUSTED_SERVERS_KEY, [...trusted]);
	}

	async signIn(server: string, ticket: string): Promise<TokenPair> {
		const pair = await new ServerApi(server).redeemTicket(ticket);
		await this.store(pair.env, server, pair);
		const map = { ...this.ctx.globalState.get<Record<string, string>>(ENV_SERVERS_KEY, {}), [pair.env]: server };
		await this.ctx.globalState.update(ENV_SERVERS_KEY, map);
		this.log.info(`signed in to ${server} as ${pair.user} for env ${pair.env}`);
		return pair;
	}

	private async store(env: string, server: string, p: TokenPair): Promise<StoredTokens> {
		const now = Date.now();
		const t: StoredTokens = {
			server,
			user: p.user,
			accessToken: p.accessToken,
			accessExpiresAt: now + p.accessExpiresIn * 1000,
			refreshToken: p.refreshToken,
			refreshExpiresAt: now + p.refreshExpiresIn * 1000,
		};
		await this.ctx.secrets.store(secretKey(env), JSON.stringify(t));
		return t;
	}

	private async load(env: string): Promise<StoredTokens | undefined> {
		const raw = await this.ctx.secrets.get(secretKey(env));
		return raw ? (JSON.parse(raw) as StoredTokens) : undefined;
	}

	/**
	 * Like `load`, but waits a little for tokens that another window just stored: the window
	 * that handled the launch link signs in and opens this window right away, and secrets
	 * written there can take a moment to become visible here.
	 */
	private async loadOrWait(env: string, timeoutMs = 10_000): Promise<StoredTokens | undefined> {
		const t = await this.load(env);
		if (t) {
			return t;
		}
		this.log.info(`no tokens for ${env} yet; waiting up to ${timeoutMs / 1000} s for the sign-in from another window`);
		const started = Date.now();
		return new Promise(resolve => {
			let done = false;
			const finish = (v: StoredTokens | undefined) => {
				if (done) {
					return;
				}
				done = true;
				sub.dispose();
				clearInterval(poll);
				clearTimeout(timer);
				if (v) {
					this.log.info(`tokens for ${env} arrived after ${Date.now() - started} ms`);
				}
				resolve(v);
			};
			const check = () => void this.load(env).then(v => v && finish(v), () => {});
			const sub = this.ctx.secrets.onDidChange(e => e.key === secretKey(env) && check());
			const poll = setInterval(check, 500);
			const timer = setTimeout(() => finish(undefined), timeoutMs);
		});
	}

	async user(env: string): Promise<string | undefined> {
		return (await this.load(env))?.user;
	}

	/** A valid access token, refreshing when it is about to expire. `force` refreshes regardless. */
	async accessToken(env: string, force = false): Promise<{ server: string; token: string }> {
		const t = await this.loadOrWait(env);
		if (!t) {
			throw new ServerError(401, 'signed_out', `Not signed in to environment "${env}".`);
		}
		if (!force && t.accessExpiresAt - Date.now() > 60_000) {
			return { server: t.server, token: t.accessToken };
		}
		let p = this.refreshing.get(env);
		if (!p) {
			p = this.doRefresh(env, t).finally(() => this.refreshing.delete(env));
			this.refreshing.set(env, p);
		}
		const fresh = await p;
		return { server: fresh.server, token: fresh.accessToken };
	}

	private async doRefresh(env: string, t: StoredTokens): Promise<StoredTokens> {
		// Another window may have refreshed already.
		const latest = (await this.load(env)) ?? t;
		if (latest.accessToken !== t.accessToken && latest.accessExpiresAt - Date.now() > 60_000) {
			return latest;
		}
		this.log.debug(`refreshing tokens for ${env}`);
		try {
			const pair = await new ServerApi(latest.server).refresh(latest.refreshToken);
			return await this.store(env, latest.server, pair);
		} catch (e) {
			if (e instanceof ServerError && e.isAuthFailure) {
				await this.ctx.secrets.delete(secretKey(env));
			}
			throw e;
		}
	}

	async signOut(env: string): Promise<void> {
		const t = await this.load(env);
		await this.ctx.secrets.delete(secretKey(env));
		if (t) {
			try {
				await new ServerApi(t.server).revokeTokens(t.refreshToken);
			} catch (e) {
				this.log.warn(`revoking tokens failed: ${(e as Error).message}`);
			}
		}
		this.log.info(`signed out of ${env}`);
	}
}

export function normalize(url: string): string {
	return url.trim().replace(/\/+$/, '').toLowerCase();
}
