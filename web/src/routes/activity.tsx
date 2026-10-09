import { createFileRoute } from "@tanstack/react-router"
import { useState } from "react"
import { ActorBadge, Empty, PageHeader } from "@/components/bits"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent } from "@/components/ui/card"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useActivity } from "@/lib/api"
import { ago } from "@/lib/format"

export const Route = createFileRoute("/activity")({ component: ActivityPage })

function ActivityPage() {
  const [kind, setKind] = useState("all")
  const { data = [] } = useActivity(kind)
  return (
    <>
      <PageHeader title="Activity" description="Append-only log of every sync, organise action, proposal and decision." />
      <Tabs value={kind} onValueChange={setKind} className="mb-4">
        <TabsList>
          <TabsTrigger value="all">All</TabsTrigger>
          <TabsTrigger value="agent">Agents</TabsTrigger>
          <TabsTrigger value="human">Humans</TabsTrigger>
          <TabsTrigger value="system">System</TabsTrigger>
        </TabsList>
      </Tabs>
      <Card className="py-0 shadow-none">
        <CardContent className="divide-y p-0">
          {data.map((a) => (
            <div key={a.id} className="flex items-start gap-4 px-5 py-3">
              <div className="w-40 shrink-0">
                <ActorBadge kind={a.actor_kind} name={a.actor} />
              </div>
              <div className="min-w-0 flex-1 text-sm">{a.summary}</div>
              <Badge variant="outline" className="shrink-0 font-normal">
                {a.action}
              </Badge>
              <span className="text-muted-foreground w-20 shrink-0 text-right text-xs" title={a.created_at}>
                {ago(a.created_at)}
              </span>
            </div>
          ))}
          {!data.length && <Empty>No activity yet.</Empty>}
        </CardContent>
      </Card>
    </>
  )
}
