import { Link, useLocation } from "@tanstack/react-router";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

export function NotFoundPage() {
  const pathname = useLocation({ select: (location) => location.pathname });
  return (
    <PageHeader
      eyebrow={strings.notFound.eyebrow}
      title={strings.notFound.title}
      lede={strings.notFound.lede(pathname)}
    >
      <div className="mt-3">
        <Button asChild variant="outline">
          <Link to="/">{strings.notFound.home}</Link>
        </Button>
      </div>
    </PageHeader>
  );
}
