import { Link } from "@tanstack/react-router";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

/** In place of a page the signed-in user's role does not open. */
export function NotAllowedPage() {
  return (
    <PageHeader
      eyebrow={strings.notAllowed.eyebrow}
      title={strings.notAllowed.title}
      lede={strings.notAllowed.lede}
    >
      <div className="mt-3">
        <Button asChild variant="outline">
          <Link to="/">{strings.notAllowed.home}</Link>
        </Button>
      </div>
    </PageHeader>
  );
}
