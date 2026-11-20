import React from 'react';

interface Neo4jLogoProps {
  size?: number;
}

/**
 * Relating graph mark in the Neo4j idiom: three nodes joined by
 * relationships, drawn in white/Neo4j-blue for dark-navy surfaces.
 */
export const Neo4jLogo: React.FC<Neo4jLogoProps> = ({ size = 32 }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect width="32" height="32" rx="7" fill="#018bff" />
    <line x1="9" y1="21" x2="16" y2="12" stroke="#fff" strokeWidth="1.8" opacity="0.85" />
    <line x1="16" y1="12" x2="23.5" y2="19" stroke="#fff" strokeWidth="1.8" opacity="0.85" />
    <circle cx="9" cy="21" r="3.4" fill="#fff" />
    <circle cx="16" cy="12" r="4.2" fill="#0b2237" stroke="#fff" strokeWidth="1.8" />
    <circle cx="23.5" cy="19" r="3.4" fill="#fff" />
  </svg>
);
