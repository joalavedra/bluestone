import { createFileRoute } from "@tanstack/react-router"
import { RefreshCw } from "lucide-react"
import { toast } from "sonner"
import { ChannelBadge, Empty, PageHeader } from "@/components/bits"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Badge } from "@/components/ui/badge"
import { post, useAction, useBrands, useWarehouses } from "@/lib/api"
import type { BrandSummary } from "@/lib/api"
import { ago } from "@/lib/format"

export const Route = createFileRoute("/channels")({ component: ChannelsPage })

function Snippet({ children }: { children: string }) {
  return (
    <pre className="overflow-x-auto rounded-lg bg-slate-950 p-4 text-xs leading-relaxed text-slate-100">
      {children}
    </pre>
  )
}

function StockModeControl({ brand }: { brand: BrandSummary }) {
  const toggle = useAction((mode: string) =>
    post(`/brands/${brand.id}/stock-mode`, { mode })
  )
  const master = brand.stock_mode === "master"
  const next = master ? "mirror" : "master"
  const confirmText = master
    ? `Switch ${brand.name} back to mirror mode? The stores will own stock again; the ledger is kept as history.`
    : `Make Bluestone the stock master for ${brand.name}? Current channel stock becomes the ledger baseline, and from now on Bluestone pushes quantities to every channel.`
  return (
    <div className="flex items-center gap-2">
      <Badge variant={master ? "default" : "secondary"}>
        {master ? "Bluestone is stock master" : "Mirroring stores"}
      </Badge>
      <Button
        variant="outline"
        size="sm"
        disabled={toggle.isPending}
        onClick={() => {
          if (!window.confirm(confirmText)) return
          toggle.mutate(next, {
            onSuccess: (r: any) =>
              toast.success(
                next === "master"
                  ? `${brand.name} is now master · ${r?.reconcile?.pushed ?? 0} channel updates`
                  : `${brand.name} mirrors its stores again`
              ),
            onError: (e) => toast.error(e.message),
          })
        }}
      >
        {master ? "Switch to mirror" : "Make master"}
      </Button>
    </div>
  )
}

function WarehouseList({ brand }: { brand: string }) {
  const { data = [] } = useWarehouses()
  const mine = data.filter((w) => w.brand === brand)
  if (!mine.length) return null
  return (
    <div className="mt-4 flex flex-wrap gap-2">
      {mine.map((w) => (
        <div
          key={w.id}
          className="rounded-lg border bg-slate-50 px-3 py-2 text-sm"
        >
          <div className="font-medium">{w.name}</div>
          <div className="text-xs text-muted-foreground">
            {w.units} units · {w.locations ?? "no channel locations"}
          </div>
        </div>
      ))}
    </div>
  )
}

function ChannelsPage() {
  const { data = [] } = useBrands()
  const syncAll = useAction(() => post("/sync"))
  const syncOne = useAction((id: number) => post(`/channels/${id}/sync`))
  const origin =
    typeof window === "undefined"
      ? "http://127.0.0.1:8787"
      : window.location.origin.replace(/:\d+$/, ":8787")
  return (
    <>
      <PageHeader
        title="Channels"
        description="Connected Shopify and PrestaShop stores, and how to plug in your agents."
        actions={
          <Button
            disabled={syncAll.isPending}
            onClick={() =>
              syncAll.mutate(undefined, {
                onSuccess: () => toast.success("Synced all channels"),
                onError: (e) => toast.error(e.message),
              })
            }
          >
            <RefreshCw
              className={syncAll.isPending ? "size-4 animate-spin" : "size-4"}
            />{" "}
            Sync all
          </Button>
        }
      />
      <div className="flex flex-col gap-4">
        {data.map((b) => (
          <Card key={b.id} className="shadow-none">
            <CardHeader>
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div>
                  <CardTitle>{b.name}</CardTitle>
                  <CardDescription>
                    {b.item_count} items · {b.low_stock_count} low ·{" "}
                    {b.out_of_stock_count} out
                  </CardDescription>
                </div>
                <StockModeControl brand={b} />
              </div>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Channel</TableHead>
                    <TableHead>URL</TableHead>
                    <TableHead className="text-right">Listings</TableHead>
                    <TableHead>Last sync</TableHead>
                    <TableHead />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {b.channels.map((c) => (
                    <TableRow key={c.id}>
                      <TableCell>
                        <ChannelBadge channel={`${c.kind}:${c.name}`} />
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {c.base_url}
                      </TableCell>
                      <TableCell className="text-right tabular-nums">
                        {c.listing_count}
                      </TableCell>
                      <TableCell>
                        {ago(c.last_synced_at)}
                        {c.last_sync_error && (
                          <div
                            className="max-w-sm truncate text-xs text-red-600"
                            title={c.last_sync_error}
                          >
                            {c.last_sync_error}
                          </div>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={syncOne.isPending}
                          onClick={() =>
                            syncOne.mutate(c.id, {
                              onSuccess: () =>
                                toast.success(`Synced ${c.name}`),
                              onError: (e) => toast.error(e.message),
                            })
                          }
                        >
                          Sync now
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
              {b.stock_mode === "master" && <WarehouseList brand={b.name} />}
            </CardContent>
          </Card>
        ))}
        {!data.length && (
          <Empty>
            No channels yet. Add one with <code>bluestone channel add</code>.
          </Empty>
        )}
        <Card className="shadow-none">
          <CardHeader>
            <CardTitle>Connect an agent</CardTitle>
            <CardDescription>
              Create a scoped agent token, then point Claude Code or your
              harness at Bluestone's MCP server.
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4 lg:grid-cols-2">
            <div>
              <div className="mb-2 text-sm font-medium">
                Claude Code (stdio)
              </div>
              <Snippet>{`bluestone token create --name claude-code
claude mcp add bluestone \\
  --env BLUESTONE_TOKEN=bst_… \\
  --env BLUESTONE_DATABASE=sqlite:///path/to/bluestone.db \\
  -- /path/to/bluestone mcp`}</Snippet>
            </div>
            <div>
              <div className="mb-2 text-sm font-medium">
                Custom agent (streamable HTTP)
              </div>
              <Snippet>{`{
  "mcpServers": {
    "bluestone": {
      "type": "http",
      "url": "${origin}/mcp",
      "headers": { "Authorization": "Bearer bst_…" }
    }
  }
}`}</Snippet>
            </div>
          </CardContent>
        </Card>
      </div>
    </>
  )
}
