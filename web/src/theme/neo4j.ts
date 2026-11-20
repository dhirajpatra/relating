/**
 * Relating Neo4j theme — one place for the palette and shared shapes.
 *
 * The look mirrors the Neo4j Aura walkthrough slides: a deep-navy page,
 * Neo4j-blue accents (#018bff), and white diagram cards with dark-navy ink.
 * Components stay CSS-variable driven so the theme can be re-skinned from
 * `web/index.html` alone; this module only names the tokens and bundles the
 * shapes every panel repeats (card, button, input, pill).
 */

export const neo4jTheme = {
  colors: {
    /** Deep navy page background. */
    bgPrimary: 'var(--bg-primary)',
    /** Navy card surface. */
    bgSecondary: 'var(--bg-secondary)',
    bgHover: 'var(--bg-hover)',
    border: 'var(--border-color)',
    /** White ink on navy. */
    textPrimary: 'var(--text-primary)',
    textSecondary: 'var(--text-secondary)',
    textMuted: 'var(--text-muted)',
    /** Neo4j blue. */
    accent: 'var(--accent)',
    accentHover: 'var(--accent-hover)',
    accentLight: 'var(--accent-light)',
    /** White diagram-card surface (dark ink goes on top). */
    heroCard: 'var(--hero-card)',
    /** Dark-navy ink for white surfaces. */
    heroInk: 'var(--hero-ink)',
    success: 'var(--success)',
    warning: 'var(--warning)',
    danger: 'var(--danger)',
  },
  /** Soft accent wash for chips and selected rows. */
  accentSoft: (alpha: number) => `rgba(var(--accent-rgb), ${alpha})`,
  radius: {
    sm: 4,
    md: 6,
    lg: 10,
    xl: 12,
    pill: 9999,
  },
} as const;

/** Navy panel card shared by every workbench section. */
export const cardStyle: React.CSSProperties = {
  backgroundColor: 'var(--bg-secondary)',
  borderRadius: neo4jTheme.radius.lg,
  border: '1px solid var(--border-color)',
  padding: '1.25rem',
};

/** Primary Neo4j-blue button. */
export const primaryButtonStyle: React.CSSProperties = {
  backgroundColor: 'var(--accent)',
  color: '#fff',
  border: 'none',
  borderRadius: neo4jTheme.radius.md,
  padding: '0.5rem 1rem',
  fontWeight: 600,
  fontSize: '0.875rem',
};

/** Ghost button for secondary actions. */
export const ghostButtonStyle: React.CSSProperties = {
  backgroundColor: 'transparent',
  color: 'var(--text-secondary)',
  border: '1px solid var(--border-color)',
  borderRadius: neo4jTheme.radius.md,
  padding: '0.4rem 0.9rem',
  fontSize: '0.8rem',
  fontWeight: 600,
};

/** Dark navy text input shared by forms. */
export const inputStyle: React.CSSProperties = {
  padding: '0.5rem 0.7rem',
  backgroundColor: 'var(--bg-primary)',
  border: '1px solid var(--border-color)',
  borderRadius: neo4jTheme.radius.md,
  color: 'var(--text-primary)',
  fontSize: '0.85rem',
};
