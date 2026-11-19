import * as vscode from 'vscode';
import { BrokerApi, BrokerError, TokenPair } from './brokerApi';

interface StoredTokens {
	broker: string;
	user: string;
	accessToken: string;
	accessExpiresAt: number;
	refreshToken: string;
	refreshExpiresAt: number;
}

const ENV_BROKERS_KEY = 'opentunnel.envBrokers';
const TRUSTED_BROKERS_KEY = 'opentunnel.trustedBrokers';
const secretKey = (env: string) => `opentunnel.tokens.${env}`;

/** Per-environment client tokens in SecretStorage, refreshed on demand. */
export class AuthStore {
	private refreshing = new Map<string, Promise<StoredTokens>>();

	constructor(private readonly ctx: vscode.ExtensionContext, private readonly log: vscode.LogOutputChannel) {}

	brokerFor(env: string): string | undefined {
		const map = this.ctx.globalState.get<Record<string, string>>(ENV_BROKERS_KEY, {});
		return map[env] || vscode.workspace.getConfiguration('opentunnel').get<string>('brokerUrl') || undefined;
	}

	isTrustedBroker(url: string): boolean {
		const configured = vscode.workspace.getConfiguration('opentunnel').get<string>('brokerUrl');
		const trusted = this.ctx.globalState.get<string[]>(TRUSTED_BROKERS_KEY, []);
		return normalize(url) === normalize(configured ?? '') || trusted.includes(normalize(url));
	}

	async trustBroker(url: string): Promise<void> {
		const trusted = new Set(this.ctx.globalState.get<string[]>(TRUSTED_BROKERS_KEY, []));
		trusted.add(normalize(url));
		await this.ctx.globalState.update(TRUSTED_BROKERS_KEY, [...trusted]);
	}

	async signIn(broker: string, ticket: string): Promise<TokenPair> {
		const pair = await new BrokerApi(broker).redeemTicket(ticket);
		await this.store(pair.env, broker, pair);
		const map = { ...this.ctx.globalState.get<Record<string, string>>(ENV_BROKERS_KEY, {}), [pair.env]: broker };
		await this.ctx.globalState.update(ENV_BROKERS_KEY, map);
		this.log.info(`signed in to ${broker} as ${pair.user} for env ${pair.env}`);
		return pair;
	}

	private async store(env: string, broker: string, p: TokenPair): Promise<StoredTokens> {
		const now = Date.now();
		const t: StoredTokens = {
			broker,
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

	async user(env: string): Promise<string | undefined> {
		return (await this.load(env))?.user;
	}

	/** A valid access token, refreshing when it is about to expire. `force` refreshes regardless. */
	async accessToken(env: string, force = false): Promise<{ broker: string; token: string }> {
		const t = await this.load(env);
		if (!t) {
			throw new BrokerError(401, 'signed_out', `Not signed in to environment "${env}".`);
		}
		if (!force && t.accessExpiresAt - Date.now() > 60_000) {
			return { broker: t.broker, token: t.accessToken };
		}
		let p = this.refreshing.get(env);
		if (!p) {
			p = this.doRefresh(env, t).finally(() => this.refreshing.delete(env));
			this.refreshing.set(env, p);
		}
		const fresh = await p;
		return { broker: fresh.broker, token: fresh.accessToken };
	}

	private async doRefresh(env: string, t: StoredTokens): Promise<StoredTokens> {
		// Another window may have refreshed already.
		const latest = (await this.load(env)) ?? t;
		if (latest.accessToken !== t.accessToken && latest.accessExpiresAt - Date.now() > 60_000) {
			return latest;
		}
		this.log.debug(`refreshing tokens for ${env}`);
		try {
			const pair = await new BrokerApi(latest.broker).refresh(latest.refreshToken);
			return await this.store(env, latest.broker, pair);
		} catch (e) {
			if (e instanceof BrokerError && e.isAuthFailure) {
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
				await new BrokerApi(t.broker).revokeTokens(t.refreshToken);
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
