import { Link } from "@tanstack/react-router";
import { Paintbrush, Shapes } from "lucide-react";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

/** The empty state until there are pages to list. */
export function HomePage() {
  return (
    <PageHeader eyebrow={strings.home.eyebrow} title={strings.home.title} lede={strings.home.lede}>
      <div className="mt-3 flex flex-wrap gap-2">
        <Button asChild>
          <Link to="/settings/appearance">
            <Paintbrush aria-hidden />
            {strings.home.appearance}
          </Link>
        </Button>
        <Button asChild variant="outline">
          <Link to="/styleguide">
            <Shapes aria-hidden />
            {strings.home.styleguide}
          </Link>
        </Button>
      </div>
    </PageHeader>
  );
}
