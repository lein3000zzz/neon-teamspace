import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import type { AppRouter } from "./router";
import { ThemeProvider } from "./theme/theme-provider";

type AppProps = {
  queryClient: QueryClient;
  router: AppRouter;
};

export function App({ queryClient, router }: AppProps) {
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <RouterProvider router={router} />
      </ThemeProvider>
    </QueryClientProvider>
  );
}
