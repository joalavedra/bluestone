import { Badge } from "@/components/ui/badge"
import { Card, CardContent } from "@/components/ui/card"
import { cn } from "@/lib/utils"
import type { Status } from "@/lib/api"
import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"

const statusStyle: Record<Status, { label: string; className: string }> = {
  out: { label: "Out of stock", className: "bg-red-50 text-red-700 border-red-200" },
  low: { label: "Low", className: "bg-amber-50 text-amber-700 border-amber-200" },
  ok: { label: "Healthy", className: "bg-emerald-50 text-emerald-700 border-emerald-200" },
}

export function StatusBadge({ status }: { status: Status }) {
  const s = statusStyle[status]
  return (
    <Badge variant="outline" className={cn("font-medium", s.className)}>
      {s.label}
    </Badge>
  )
}

export function ChannelBadge({ channel }: { channel: string }) {
  const [kind, name] = channel.includes(":") ? channel.split(":") : [channel, channel]
  return (
    <Badge
      variant="outline"
      className={cn(
        "gap-1 font-normal",
        kind === "shopify"
          ? "border-emerald-200 bg-emerald-50/60 text-emerald-800"
          : kind === "faire"
            ? "border-amber-200 bg-amber-50/60 text-amber-800"
            : "border-pink-200 bg-pink-50/60 text-pink-800",
      )}
    >
      <span className={cn("size-1.5 rounded-full", kind === "shopify" ? "bg-emerald-500" : kind === "faire" ? "bg-amber-500" : "bg-pink-500")} />
      {name}
    </Badge>
  )
}

export function ActorBadge({ kind, name }: { kind: string; name: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-sm">
      <span
        className={cn(
          "inline-flex size-5 items-center justify-center rounded-full text-[10px] font-semibold uppercase",
          kind === "agent" ? "bg-primary/10 text-primary" : kind === "human" ? "bg-slate-900 text-white" : "bg-slate-200 text-slate-600",
        )}
      >
        {kind === "agent" ? "AI" : name.slice(0, 1)}
      </span>
      <span className="font-medium">{name}</span>
    </span>
  )
}

export function Kpi({
  label,
  value,
  hint,
  icon: Icon,
  tone = "default",
}: {
  label: string
  value: ReactNode
  hint?: ReactNode
  icon?: LucideIcon
  tone?: "default" | "warn" | "bad" | "accent"
}) {
  return (
    <Card className="gap-0 py-0 shadow-none">
      <CardContent className="flex items-start justify-between p-4">
        <div>
          <div className="text-muted-foreground text-xs font-medium">{label}</div>
          <div
            className={cn(
              "mt-1 text-2xl font-semibold tracking-tight",
              tone === "warn" && "text-amber-600",
              tone === "bad" && "text-red-600",
              tone === "accent" && "text-primary",
            )}
          >
            {value}
          </div>
          {hint && <div className="text-muted-foreground mt-0.5 text-xs">{hint}</div>}
        </div>
        {Icon && (
          <div className="bg-secondary text-primary rounded-lg p-2">
            <Icon className="size-4" />
          </div>
        )}
      </CardContent>
    </Card>
  )
}

export function PageHeader({ title, description, actions }: { title: string; description?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="mb-6 flex flex-wrap items-end justify-between gap-3">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        {description && <p className="text-muted-foreground mt-1 text-sm">{description}</p>}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </div>
  )
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="text-muted-foreground py-10 text-center text-sm">{children}</div>
}

export function Thumb({ src, alt }: { src: string | null; alt: string }) {
  return src ? (
    <img src={src} alt={alt} className="size-9 rounded-md border object-cover" />
  ) : (
    <div className="bg-muted size-9 rounded-md border" />
  )
}
