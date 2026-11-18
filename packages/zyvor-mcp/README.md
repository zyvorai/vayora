# @zyvor/mcp (Vayora MCP server)

A [Model Context Protocol](https://modelcontextprotocol.io) server over `@zyvor/node`: read-only by default, with a few
opt-in write tools that still go through Vayora's own approval policy. Local stdio only. Not published to npm.

```sh
pnpm --filter @zyvor/node --filter @zyvor/mcp build
ZYVOR_API_KEY=zk_test_... ZYVOR_API_URL=http://localhost:8080 node packages/zyvor-mcp/dist/cli.js   # an MCP client starts this for you
ZYVOR_MCP_ENABLE_WRITES=1 ...                                                                       # also registers the write tools
```

The key comes only from `ZYVOR_API_KEY`. Setup for common clients, the tool list with scopes, the threat model and the
limits are in [docs/zyvor/AI_INTEGRATION.md](../../docs/zyvor/AI_INTEGRATION.md). A scripted sandbox demo is
`pnpm --filter @zyvor/mcp demo` (needs a local sandbox and a test key).
