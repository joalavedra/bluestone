import { Link, useRouterState } from "@tanstack/react-router"
import { Activity, Boxes, ClipboardList, Inbox, LayoutDashboard, LogOut, Plug, ReceiptText, ShoppingCart } from "lucide-react"
import { useEffect, useState } from "react"
import type { ReactNode } from "react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { AuthError, getToken, setToken, useMe, useOverview } from "@/lib/api"
import { cn } from "@/lib/utils"

const nav = [
  { to: "/", label: "Overview", icon: LayoutDashboard },
  { to: "/inventory", label: "Inventory", icon: Boxes },
  { to: "/approvals", label: "Approvals", icon: Inbox },
  { to: "/reorder", label: "Reorder", icon: ShoppingCart },
  { to: "/purchase-orders", label: "Purchase orders", icon: ClipboardList },
  { to: "/sales-orders", label: "Sales orders", icon: ReceiptText },
  { to: "/activity", label: "Activity", icon: Activity },
  { to: "/channels", label: "Channels", icon: Plug },
] as const

function Logo() {
  return (
    <div className="flex items-center gap-2 px-3 py-1">
      <div className="bg-primary flex size-7 items-center justify-center rounded-lg text-sm font-bold text-white">B</div>
      <span className="text-[15px] font-semibold tracking-tight">Bluestone</span>
    </div>
  )
}

function Sidebar() {
  const path = useRouterState({ select: (s) => s.location.pathname })
  const me = useMe()
  const overview = useOverview()
  const pending = overview.data?.pending_proposals ?? 0
  return (
    <aside className="sticky top-0 flex h-screen w-60 shrink-0 flex-col border-r bg-white px-3 py-4">
      <Logo />
      <nav className="mt-6 flex flex-col gap-0.5">
        {nav.map(({ to, label, icon: Icon }) => {
          const active = to === "/" ? path === "/" : path.startsWith(to) || (to === "/inventory" && path.startsWith("/items"))
          return (
            <Link
              key={to}
              to={to}
              className={cn(
                "flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
                active ? "bg-secondary text-secondary-foreground" : "text-slate-600 hover:bg-slate-50 hover:text-slate-900",
              )}
            >
              <Icon className="size-4" />
              <span className="flex-1">{label}</span>
              {to === "/approvals" && pending > 0 && (
                <span className="bg-primary rounded-full px-1.5 py-0.5 text-[11px] leading-none font-semibold text-white">{pending}</span>
              )}
            </Link>
          )
        })}
      </nav>
      <div className="mt-auto flex items-center justify-between rounded-lg border px-3 py-2">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium">{me.data?.name ?? "…"}</div>
          <div className="text-muted-foreground text-xs">{me.data?.kind} token</div>
        </div>
        <Button variant="ghost" size="icon" aria-label="Sign out" onClick={() => setToken(null)}>
          <LogOut className="size-4" />
        </Button>
      </div>
    </aside>
  )
}

function TokenGate() {
  const [value, setValue] = useState("")
  return (
    <div className="flex min-h-screen items-center justify-center p-6">
      <Card className="w-full max-w-md">
        <CardHeader>
          <Logo />
          <CardTitle className="mt-4">Sign in</CardTitle>
          <CardDescription>
            Paste a human token. Create one with{" "}
            <code className="bg-muted rounded px-1 text-xs">bluestone token create --name you --kind human --scopes admin</code>
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="flex flex-col gap-3"
            onSubmit={(e) => {
              e.preventDefault()
              if (value.trim()) setToken(value.trim())
            }}
          >
            <Label htmlFor="token">Token</Label>
            <Input id="token" placeholder="bst_…" value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
            <Button type="submit">Continue</Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

function Authed({ children }: { children: ReactNode }) {
  const me = useMe()
  if (me.error instanceof AuthError) return <TokenGate />
  return (
    <div className="flex min-h-screen">
      <Sidebar />
      <main className="min-w-0 flex-1 px-8 py-7">
        <div className="mx-auto max-w-[1280px]">{children}</div>
      </main>
    </div>
  )
}

export function AppShell({ children }: { children: ReactNode }) {
  const [token, setTok] = useState<string | null | undefined>(undefined)
  useEffect(() => {
    const read = () => setTok(getToken())
    read()
    window.addEventListener("bluestone-auth", read)
    return () => window.removeEventListener("bluestone-auth", read)
  }, [])
  if (token === undefined) return null
  if (!token) return <TokenGate />
  return <Authed key={token}>{children}</Authed>
}
