import type { Metadata, Viewport } from "next";
import "./globals.css";
import { ReactFlowProvider } from "@xyflow/react";
import { Provider } from "jotai";
import type { ReactNode } from "react";
import { ThemeProvider } from "@/components/theme-provider";
import { AuthProvider } from "@/components/auth/auth-provider";
import { Toaster } from "@/components/ui/sonner";
import { mono, sans } from "@/lib/fonts";
import { cn } from "@/lib/utils";

export const metadata: Metadata = {
  title: "DeFi Flow — Visual Strategy Builder",
  description:
    "Build DeFi strategies visually. Drag-and-drop nodes for perps, lending, LPs, options, bridges and more. Export valid strategy JSON for the DeFi Flow engine.",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  maximumScale: 1,
  userScalable: false,
  viewportFit: "cover",
};

const RootLayout = ({ children }: { children: ReactNode }) => (
  <html lang="en" suppressHydrationWarning>
    <body className={cn(sans.variable, mono.variable, "antialiased")}>
      <ThemeProvider
        attribute="class"
        defaultTheme="dark"
        disableTransitionOnChange
        enableSystem
      >
        <Provider>
          <AuthProvider>
            <ReactFlowProvider>{children}</ReactFlowProvider>
            <Toaster />
          </AuthProvider>
        </Provider>
      </ThemeProvider>
    </body>
  </html>
);

export default RootLayout;
