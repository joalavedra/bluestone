import { Link, createFileRoute, useNavigate } from "@tanstack/react-router"
import { AlertTriangle, Boxes, Euro, Inbox, PackageX, TrendingUp } from "lucide-react"
import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts"
import { ActorBadge, ChannelBadge, Empty, Kpi, PageHeader, StatusBadge, Thumb } from "@/components/bits"
import { Button } from "@/components/ui/button"
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { useOverview, useSales } from "@/lib/api"
import { ago, compactMoney, num, shortDay } from "@/lib/format"

export const Route = createFileRoute("/")({ component: OverviewPage })

function OverviewPage() {
  const { data } = useOverview()
  const sales = useSales(30)
  const navigate = useNavigate()
  if (!data) return null
  return (
    <>
      <PageHeader title="Overview" description="Everything your agent and you see across Shopify and PrestaShop." />
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        <Kpi label="Items" value={num(data.items)} hint={`${data.brands.length} brands`} icon={Boxes} />
        <Kpi label="Low stock" value={num(data.low_stock)} tone={data.low_stock ? "warn" : "default"} icon={AlertTriangle} />
        <Kpi label="Out of stock" value={num(data.out_of_stock)} tone={data.out_of_stock ? "bad" : "default"} icon={PackageX} />
        <Kpi label="Pending approvals" value={num(data.pending_proposals)} tone={data.pending_proposals ? "accent" : "default"} icon={Inbox} />
        <Kpi label="Units sold · 30d" value={num(data.units_30d)} icon={TrendingUp} />
        <Kpi label="Revenue · 30d" value={compactMoney(data.revenue_30d)} hint={`stock value ${compactMoney(data.stock_value)}`} icon={Euro} />
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-3">
        <Card className="shadow-none lg:col-span-2">
          <CardHeader>
            <CardTitle>Units sold</CardTitle>
            <CardDescription>Last 30 days, all channels</CardDescription>
          </CardHeader>
          <CardContent className="h-56">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={sales.data?.by_day ?? []} margin={{ left: -20, right: 8 }}>
                <defs>
                  <linearGradient id="fillUnits" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="var(--chart-1)" stopOpacity={0.25} />
                    <stop offset="100%" stopColor="var(--chart-1)" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid vertical={false} stroke="var(--border)" />
                <XAxis dataKey="day" tickFormatter={shortDay} tickLine={false} axisLine={false} fontSize={11} minTickGap={24} />
                <YAxis tickLine={false} axisLine={false} fontSize={11} allowDecimals={false} />
                <Tooltip labelFormatter={(d) => shortDay(String(d))} />
                <Area dataKey="units" type="monotone" stroke="var(--chart-1)" strokeWidth={2} fill="url(#fillUnits)" />
              </AreaChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>
        <Card className="shadow-none">
          <CardHeader>
            <CardTitle>Brands & channels</CardTitle>
            <CardAction>
              <Button variant="link" size="sm" asChild>
                <Link to="/channels">Manage</Link>
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            {data.brands.map((b) => (
              <div key={b.id} className="rounded-lg border p-3">
                <div className="flex items-center justify-between">
                  <span className="font-medium">{b.name}</span>
                  <span className="text-muted-foreground text-xs">{b.item_count} items</span>
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {b.channels.map((c) => (
                    <ChannelBadge key={c.id} channel={`${c.kind}:${c.name}`} />
                  ))}
                </div>
                <div className="text-muted-foreground mt-2 text-xs">
                  {b.low_stock_count} low · {b.out_of_stock_count} out
                </div>
              </div>
            ))}
            {!data.brands.length && <Empty>No brands yet — connect a channel.</Empty>}
          </CardContent>
        </Card>
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-3">
        <Card className="shadow-none lg:col-span-2">
          <CardHeader>
            <CardTitle>Needs attention</CardTitle>
            <CardDescription>Out of stock or below lead-time cover, most urgent first</CardDescription>
            <CardAction>
              <Button variant="link" size="sm" asChild>
                <Link to="/reorder">Reorder list</Link>
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Item</TableHead>
                  <TableHead className="text-right">On hand</TableHead>
                  <TableHead className="text-right">Velocity</TableHead>
                  <TableHead className="text-right">Days cover</TableHead>
                  <TableHead>Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.attention.map((i) => (
                  <TableRow key={i.id} className="cursor-pointer" onClick={() => navigate({ to: "/items/$itemId", params: { itemId: String(i.id) } })}>
                    <TableCell>
                      <div className="flex items-center gap-3">
                        <Thumb src={i.image_url} alt={i.name} />
                        <div>
                          <div className="font-medium">{i.name}</div>
                          <div className="text-muted-foreground text-xs">{i.sku}</div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="text-right tabular-nums">{num(i.on_hand)}</TableCell>
                    <TableCell className="text-right tabular-nums">{num(i.daily_velocity, 1)}/d</TableCell>
                    <TableCell className="text-right tabular-nums">{i.days_cover === null ? "—" : `${num(i.days_cover, 1)}d`}</TableCell>
                    <TableCell>
                      <StatusBadge status={i.status} />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            {!data.attention.length && <Empty>Nothing needs attention.</Empty>}
          </CardContent>
        </Card>
        <Card className="shadow-none">
          <CardHeader>
            <CardTitle>Recent activity</CardTitle>
            <CardAction>
              <Button variant="link" size="sm" asChild>
                <Link to="/activity">All</Link>
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            {data.recent_activity.map((a) => (
              <div key={a.id} className="flex flex-col gap-0.5 border-b pb-3 last:border-0">
                <div className="flex items-center justify-between">
                  <ActorBadge kind={a.actor_kind} name={a.actor} />
                  <span className="text-muted-foreground text-xs">{ago(a.created_at)}</span>
                </div>
                <p className="text-muted-foreground pl-6.5 text-sm">{a.summary}</p>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
    </>
  )
}
