type BadgeProps = { label: string };

export const Badge = ({ label }: BadgeProps) => <span>{label}</span>;

export function Badges() {
  return <Badge label="new" />;
}
