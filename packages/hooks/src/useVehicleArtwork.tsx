import React from 'react';
import { useQuery } from '@tanstack/react-query';

import { api } from './api';
import { isLocalArtworkSource } from './outboundPolicy';

type ArtworkAsset = {
  blob: Blob;
  restoring: boolean;
};

type FallbackImageProps = Omit<
  React.ImgHTMLAttributes<HTMLImageElement>,
  'src' | 'alt'
>;

type AuthenticatedVehicleArtworkProps = Omit<
  React.ImgHTMLAttributes<HTMLImageElement>,
  'src'
> & {
  source: string | null | undefined;
  fallbackSource?: string | null | undefined;
  fallbackProps?: FallbackImageProps;
  onFallbackChange?: (usingFallback: boolean) => void;
};

async function loadArtwork(source: string): Promise<ArtworkAsset> {
  const response = await api.authenticatedAsset(source);
  const restoring = response.status === 202;
  const blob = await response.blob();
  return { blob, restoring };
}

function useBlobUrl(blob: Blob | undefined) {
  const [url, setUrl] = React.useState<string | null>(null);
  React.useEffect(() => {
    const lease = blob ? { url: URL.createObjectURL(blob) } : null;
    setUrl(lease?.url ?? null);
    return () => { if (lease) URL.revokeObjectURL(lease.url); };
  }, [blob]);
  return url;
}

export function useVehicleArtwork(source: string | null | undefined) {
  const localSource = isLocalArtworkSource(source) ? source : null;
  const protectedArtwork = localSource?.startsWith('/v1/vehicle-image-cache/') === true;
  const query = useQuery({
    queryKey: ['vehicle-artwork-asset', localSource],
    queryFn: () => loadArtwork(localSource!),
    enabled: protectedArtwork,
    staleTime: Infinity,
    retry: false,
    refetchInterval: ({ state }) => state.data?.restoring === true ? 3000 : false,
  });
  const objectUrl = useBlobUrl(query.data?.blob);
  const restoring = query.data?.restoring ?? false;
  const candidates = { local: localSource, loading: null, protected: objectUrl };
  const state = protectedArtwork ? restoring ? 'loading' : 'protected' : 'local';
  return { src: candidates[state], restoring, isLoading: protectedArtwork && query.isLoading, isError: protectedArtwork && query.isError };
}

type ArtworkEvent = { kind: 'source' } | { kind: 'failed' };
function artworkFailure(_failed: boolean, event: ArtworkEvent) {
  switch (event.kind) { case 'source': return false; case 'failed': return true; }
}

export function AuthenticatedVehicleArtwork({
  source,
  fallbackSource,
  fallbackProps,
  onFallbackChange,
  alt,
  className,
  style,
  onError,
  ...props
}: AuthenticatedVehicleArtworkProps) {
  const primaryArtwork = useVehicleArtwork(source);
  const fallbackArtwork = useVehicleArtwork(fallbackSource);
  const [primaryFailed, dispatch] = React.useReducer(artworkFailure, false);

  React.useEffect(() => {
    dispatch({ kind: 'source' });
  }, [source]);

  const primaryReady = Boolean(primaryArtwork.src) && !primaryArtwork.isError;
  const selection = [primaryFailed, primaryArtwork.restoring, !primaryReady].some(Boolean) && fallbackArtwork.src ? 'fallback' : 'primary';
  const usingFallback = selection === 'fallback';
  const artwork = { primary: primaryArtwork, fallback: fallbackArtwork }[selection];

  React.useEffect(() => {
    onFallbackChange?.(usingFallback);
  }, [onFallbackChange, usingFallback]);

  if (!artwork.src) return null;

  const {
    className: fallbackClassName,
    style: fallbackStyle,
    onError: fallbackOnError,
    ...fallbackRest
  } = fallbackProps ?? {};
  const activeProps = usingFallback ? { ...props, ...fallbackRest } : props;

  const handleError: React.ReactEventHandler<HTMLImageElement> = (event) => {
    if (selection === 'primary' && fallbackArtwork.src) dispatch({ kind: 'failed' });
    const callbacks = usingFallback ? [onError, fallbackOnError] : [onError];
    callbacks.forEach((callback) => callback?.(event));
  };

  return (
    <img
      {...activeProps}
      src={artwork.src}
      alt={alt}
      className={usingFallback ? fallbackClassName ?? className : className}
      style={usingFallback ? { ...style, ...fallbackStyle } : style}
      onError={handleError}
      data-artwork-fallback={usingFallback || undefined}
      data-artwork-restoring={artwork.restoring || undefined}
    />
  );
}
