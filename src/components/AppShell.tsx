import { NavLink, Outlet } from "react-router-dom";
import { HomeIcon, SettingsIcon } from "lucide-react";
import { cn } from "@/lib/utils";

const NAV_ITEMS = [
  { to: "/", label: "Home", icon: HomeIcon, end: true },
  { to: "/settings", label: "Settings", icon: SettingsIcon, end: false },
] as const;

export function AppShell() {
  return (
    <div className="flex min-h-screen bg-background text-foreground">
      <nav
        aria-label="Primary"
        className="flex w-56 shrink-0 flex-col gap-1 border-r border-border bg-sidebar p-3 text-sidebar-foreground"
      >
        <div className="mb-4 px-2 pt-2">
          <span className="font-heading text-sm font-semibold tracking-tight">
            BISTEC Architect
          </span>
        </div>
        {NAV_ITEMS.map(({ to, label, icon: Icon, end }) => (
          <NavLink
            key={to}
            to={to}
            end={end}
            className={({ isActive }) =>
              cn(
                "flex items-center gap-2 rounded-lg px-2.5 py-2 text-sm font-medium transition-colors focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                isActive
                  ? "bg-sidebar-accent text-sidebar-accent-foreground"
                  : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
              )
            }
          >
            <Icon className="size-4" />
            {label}
          </NavLink>
        ))}
      </nav>
      <main className="min-w-0 flex-1 overflow-y-auto">
        <Outlet />
      </main>
    </div>
  );
}
