// End-to-end check of MCP OAuth sign-in (D-20261126-mcp-oauth) with the OFFICIAL MCP TypeScript SDK client against a
// real `autoxide serve`. Run by `scripts/mcp-oauth-e2e.sh` (which starts the server); not on its own.
//
// Steps, as a real MCP client and person would do them:
//  1. The client connects without a token, gets 401 + `WWW-Authenticate: … resource_metadata=…`, discovers the
//     authorization server through `/.well-known/…` and registers (dynamic registration, or a client metadata document
//     served by this script), then sends the browser to `/authorize` with PKCE S256.
//  2. This script plays the browser and the interface's consent page: it follows `/authorize` to the consent page,
//     signs the administrator in, reads the request and approves ONE read permission.
//  3. The client exchanges the code, lists tools (only what the approval allows) and calls a tool.
//  4. The access token is broken on purpose: the client refreshes with the refresh token and calls again.
//  5. The person revokes the approval: the next call fails.
//
// Environment: AUTOXIDE_URL (server root, = AUTOXIDE_HTTP_PUBLIC_URL), ADMIN_EMAIL, ADMIN_PASSWORD,
// REGISTRATION = `dynamic` | `metadata-document`.

import assert from 'node:assert/strict';
import http from 'node:http';

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { UnauthorizedError } from '@modelcontextprotocol/sdk/client/auth.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

const BASE = requireEnv('AUTOXIDE_URL').replace(/\/$/, '');
const ADMIN_EMAIL = requireEnv('ADMIN_EMAIL');
const ADMIN_PASSWORD = requireEnv('ADMIN_PASSWORD');
const REGISTRATION = process.env.REGISTRATION ?? 'dynamic';
// A loopback redirect: the server accepts any port for 127.0.0.1 (RFC 8252). Nothing listens there; the script reads
// the address the consent API answers instead of following it.
const REDIRECT = 'http://127.0.0.1:53682/callback';
const PERMISSION = 'auth.roles.read';
const TOOL = 'auth.list_roles';
const UNGRANTED_TOOL = 'auth.list_users';

function requireEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

function step(message) {
  console.log(`mcp-oauth [${REGISTRATION}]: ${message}`);
}

/** In-memory OAuthClientProvider: what a real client keeps in its own storage. */
class MemoryProvider {
  constructor(clientInformation) {
    this.client = clientInformation;
    this.saved = undefined;
    this.verifier = undefined;
    this.authorizationUrl = undefined;
  }
  get redirectUrl() {
    return REDIRECT;
  }
  get clientMetadata() {
    return {
      client_name: 'MCP SDK end-to-end check',
      redirect_uris: [REDIRECT],
      grant_types: ['authorization_code', 'refresh_token'],
      response_types: ['code'],
      token_endpoint_auth_method: 'none',
    };
  }
  state() {
    return 'state-of-the-check';
  }
  clientInformation() {
    return this.client;
  }
  saveClientInformation(information) {
    this.client = information;
  }
  tokens() {
    return this.saved;
  }
  saveTokens(tokens) {
    this.saved = tokens;
  }
  redirectToAuthorization(url) {
    this.authorizationUrl = url;
  }
  saveCodeVerifier(verifier) {
    this.verifier = verifier;
  }
  codeVerifier() {
    if (!this.verifier) throw new Error('no code verifier saved');
    return this.verifier;
  }
  // The SDK calls this when the server rejects what is stored (e.g. `invalid_grant` after a revocation).
  invalidateCredentials(scope) {
    if (scope === 'all' || scope === 'tokens') this.saved = undefined;
    if (scope === 'all' || scope === 'verifier') this.verifier = undefined;
  }
}

async function api(method, path, { token, body } = {}) {
  const headers = { accept: 'application/json' };
  if (token) headers.authorization = `Bearer ${token}`;
  if (body !== undefined) headers['content-type'] = 'application/json';
  const response = await fetch(`${BASE}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  const json = text ? JSON.parse(text) : null;
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${text}`);
  return json;
}

/** Serves the client metadata document on 127.0.0.1 (the server fetches it; private addresses allowed in this check). */
async function serveMetadataDocument() {
  const server = http.createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const clientId = `http://127.0.0.1:${server.address().port}/client.json`;
  server.on('request', (request, response) => {
    if (request.url !== '/client.json') {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(
      JSON.stringify({
        client_id: clientId,
        client_name: 'MCP SDK end-to-end check (metadata document)',
        redirect_uris: [REDIRECT],
        grant_types: ['authorization_code', 'refresh_token'],
        response_types: ['code'],
        token_endpoint_auth_method: 'none',
      }),
    );
  });
  return { server, clientId };
}

function connect(provider) {
  const transport = new StreamableHTTPClientTransport(new URL(`${BASE}/mcp`), { authProvider: provider });
  const client = new Client({ name: 'autoxide-mcp-oauth-check', version: '1.0.0' });
  return { transport, client };
}

async function main() {
  // Discovery documents a client reads (RFC 9728, RFC 8414).
  const resource = await api('GET', '/.well-known/oauth-protected-resource/mcp');
  assert.equal(resource.resource, `${BASE}/mcp`);
  assert.deepEqual(resource.authorization_servers, [BASE]);
  const metadata = await api('GET', '/.well-known/oauth-authorization-server');
  assert.equal(metadata.issuer, BASE);
  assert.deepEqual(metadata.code_challenge_methods_supported, ['S256']);
  step('discovery documents are served');

  let metadataServer;
  let provider;
  if (REGISTRATION === 'metadata-document') {
    assert.equal(metadata.client_id_metadata_document_supported, true);
    const served = await serveMetadataDocument();
    metadataServer = served.server;
    // The SDK only offers metadata-document ids over https; a client that already knows its id hands it over.
    provider = new MemoryProvider({ client_id: served.clientId });
  } else {
    provider = new MemoryProvider(undefined);
  }

  try {
    // 1. No token: the SDK discovers, registers if needed and asks for the browser.
    let { transport, client } = connect(provider);
    await assert.rejects(client.connect(transport), (error) => error instanceof UnauthorizedError);
    const authorize = provider.authorizationUrl;
    assert.ok(authorize, 'the SDK did not send the browser to /authorize');
    assert.equal(authorize.searchParams.get('code_challenge_method'), 'S256');
    assert.equal(authorize.searchParams.get('resource'), `${BASE}/mcp`);
    step(`client ${provider.client.client_id} sent to ${authorize.origin}${authorize.pathname}`);

    // 2. The browser: /authorize → the interface's consent page, where the signed-in person approves.
    const redirect = await fetch(authorize, { redirect: 'manual' });
    assert.equal(redirect.status, 302, await redirect.text());
    const consent = new URL(redirect.headers.get('location'));
    const request = consent.searchParams.get('request');
    assert.ok(request, `no request token in ${consent}`);
    const signIn = await api('POST', '/api/v1/auth/login', {
      body: { email: ADMIN_EMAIL, password: ADMIN_PASSWORD },
    });
    const session = signIn.data.access_token;
    const shown = await api('GET', `/api/v1/auth/oauth/requests/${request}`, { token: session });
    assert.equal(shown.data.redirect_host, '127.0.0.1');
    assert.equal(shown.data.localhost_redirect, true);
    assert.ok(shown.data.permissions.some((offered) => offered.name === PERMISSION));
    const decided = await api('POST', `/api/v1/auth/oauth/requests/${request}/approve`, {
      token: session,
      body: { permissions: [PERMISSION] },
    });
    const back = new URL(decided.data.redirect_to);
    assert.equal(back.searchParams.get('state'), 'state-of-the-check');
    assert.equal(back.searchParams.get('iss'), BASE);
    const code = back.searchParams.get('code');
    assert.ok(code, `no code in ${back}`);
    step(`approved ${PERMISSION}`);

    // 3. The client exchanges the code (PKCE) and calls a tool.
    await transport.finishAuth(code);
    assert.ok(provider.saved?.access_token.startsWith('axo1_'));
    assert.ok(provider.saved?.refresh_token);
    ({ transport, client } = connect(provider));
    await client.connect(transport);
    const tools = (await client.listTools()).tools.map((tool) => tool.name);
    assert.ok(tools.includes(TOOL), `tools/list lacks ${TOOL}`);
    assert.ok(!tools.includes(UNGRANTED_TOOL), `tools/list shows ${UNGRANTED_TOOL}, which was not approved`);
    let result = await client.callTool({ name: TOOL, arguments: {} });
    assert.notEqual(result.isError, true, JSON.stringify(result));
    step(`called ${TOOL} with the OAuth access token`);

    // 4. A rejected access token: the SDK refreshes (the refresh token rotates) and retries.
    const before = provider.saved.refresh_token;
    provider.saved = { ...provider.saved, access_token: 'axo1_not-a-valid-token' };
    await client.close();
    ({ transport, client } = connect(provider));
    await client.connect(transport);
    result = await client.callTool({ name: TOOL, arguments: {} });
    assert.notEqual(result.isError, true, JSON.stringify(result));
    assert.notEqual(provider.saved.refresh_token, before, 'the refresh token did not rotate');
    step('refreshed and called again');

    // 5. The person revokes the approval in the interface: the client is out at once.
    const grants = await api('GET', '/api/v1/auth/me/oauth-grants', { token: session });
    const grant = grants.data.find((row) => row.client_id === provider.client.client_id && !row.revoked_at);
    assert.ok(grant, 'the approval is not listed');
    await api('DELETE', `/api/v1/auth/me/oauth-grants/${grant.id}`, { token: session });
    await client.close();
    provider.authorizationUrl = undefined;
    ({ transport, client } = connect(provider));
    await assert.rejects(client.connect(transport), (error) => error instanceof UnauthorizedError);
    assert.ok(provider.authorizationUrl, 'after revocation the client should ask the person again');
    step('revoked: the client must ask the person again');
  } finally {
    metadataServer?.close();
  }
}

main().then(
  () => step('ok'),
  (error) => {
    console.error(error);
    process.exit(1);
  },
);
