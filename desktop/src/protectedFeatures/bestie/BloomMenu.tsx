/**
 * Bestie-scoped adaptation of Josh Puckett's MIT-licensed Bloom menu.
 * Source: https://github.com/joshpuckett/bloom/tree/8fa80a0f136b3f687c1641a09ff8c97aa8e514ba/packages/bloom
 *
 * The public package could not be pulled through the workspace's approved npm
 * proxy, so this keeps the small Root/Container/Trigger/Content surface we use
 * and points it at the repo's existing Motion runtime.
 */
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

type Direction = "top" | "bottom";
type Anchor = "start" | "center" | "end";

type BloomContextValue = {
  anchor: Anchor;
  contentRef: React.RefObject<HTMLDivElement | null>;
  direction: Direction;
  open: boolean;
  setOpen: (open: boolean) => void;
  triggerRef: React.RefObject<HTMLDivElement | null>;
};

const BloomContext = React.createContext<BloomContextValue | null>(null);

function useBloomContext() {
  const context = React.useContext(BloomContext);
  if (!context) throw new Error("Bloom components require Bloom.Root");
  return context;
}

function Root({
  anchor = "start",
  children,
  direction = "top",
  onOpenChange,
  open,
}: {
  anchor?: Anchor;
  children: React.ReactNode;
  direction?: Direction;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const triggerRef = React.useRef<HTMLDivElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (!open) return;
    const closeOnOutsidePress = (event: MouseEvent | TouchEvent) => {
      const target = event.target as Node;
      if (
        !triggerRef.current?.contains(target) &&
        !contentRef.current?.contains(target)
      ) {
        onOpenChange(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onOpenChange(false);
    };
    document.addEventListener("mousedown", closeOnOutsidePress);
    document.addEventListener("touchstart", closeOnOutsidePress);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsidePress);
      document.removeEventListener("touchstart", closeOnOutsidePress);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [onOpenChange, open]);

  const value = React.useMemo(
    () => ({
      anchor,
      contentRef,
      direction,
      open,
      setOpen: onOpenChange,
      triggerRef,
    }),
    [anchor, direction, onOpenChange, open],
  );

  return (
    <BloomContext.Provider value={value}>{children}</BloomContext.Provider>
  );
}

function Container({
  buttonSize = 48,
  children,
  className,
  edgeDraggable = false,
  menuRadius = 20,
  menuWidth = 320,
  motionDisabled = false,
  onMorphAnimationComplete,
}: {
  buttonSize?: number;
  children: React.ReactNode;
  className?: string;
  edgeDraggable?: boolean;
  menuRadius?: number;
  menuWidth?: number;
  motionDisabled?: boolean;
  onMorphAnimationComplete?: (open: boolean) => void;
}) {
  const { anchor, direction, open, setOpen } = useBloomContext();
  const reduceMotion = useReducedMotion();
  const measureRef = React.useRef<HTMLDivElement>(null);
  const [measuredHeight, setMeasuredHeight] = React.useState(buttonSize);

  React.useLayoutEffect(() => {
    if (!open || !measureRef.current) return;
    const measure = () => {
      if (measureRef.current) {
        setMeasuredHeight(measureRef.current.offsetHeight);
      }
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(measureRef.current);
    return () => observer.disconnect();
  }, [open]);

  const horizontalOffset =
    anchor === "start"
      ? 0
      : -(menuWidth - buttonSize) * (anchor === "center" ? 0.5 : 1);
  const verticalOffset =
    direction === "top" ? -buttonSize * 0.75 : buttonSize * 0.75;
  const openTransform = `translate3d(${horizontalOffset}px, ${verticalOffset}px, 0) scale(1)`;

  return (
    <div className="relative h-12 w-12">
      <motion.div
        animate={{
          borderRadius: open ? menuRadius : buttonSize / 2,
          boxShadow: open
            ? "0 20px 25px -5px rgb(0 0 0 / 0.12), 0 8px 10px -6px rgb(0 0 0 / 0.1)"
            : "0 10px 15px -3px rgb(0 0 0 / 0.12), 0 4px 6px -4px rgb(0 0 0 / 0.1)",
          height: open ? measuredHeight : buttonSize,
          transform: open ? openTransform : "translate3d(0, 0, 0) scale(1)",
          width: open ? menuWidth : buttonSize,
        }}
        className={className}
        data-testid="bestie-bloom-container"
        initial={false}
        onAnimationComplete={() => onMorphAnimationComplete?.(open)}
        onClick={(event) => {
          if (open) return;
          event.preventDefault();
          setOpen(true);
        }}
        style={{
          ...(direction === "top"
            ? { bottom: 0, left: 0 }
            : { left: 0, top: 0 }),
          cursor: open ? "default" : "pointer",
          overflow: "hidden",
          position: "absolute",
          transformOrigin: `${
            anchor === "start" ? "left" : anchor === "end" ? "right" : "center"
          } ${direction === "top" ? "bottom" : "top"}`,
          willChange: "transform",
          zIndex: open ? 50 : "auto",
        }}
        transition={
          reduceMotion || motionDisabled
            ? { duration: 0 }
            : { bounce: 0.15, type: "spring", visualDuration: 0.25 }
        }
      >
        <div ref={measureRef}>{children}</div>
        {open && edgeDraggable ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 z-10"
          >
            <div
              className="pointer-events-auto absolute inset-x-0 top-0 h-2 touch-none cursor-grab active:cursor-grabbing"
              data-bestie-drag-edge="top"
              data-bestie-drag-handle
            />
            <div
              className="pointer-events-auto absolute inset-x-0 bottom-0 h-2 touch-none cursor-grab active:cursor-grabbing"
              data-bestie-drag-edge="bottom"
              data-bestie-drag-handle
            />
            <div
              className="pointer-events-auto absolute inset-y-2 left-0 w-2 touch-none cursor-grab active:cursor-grabbing"
              data-bestie-drag-edge="left"
              data-bestie-drag-handle
            />
            <div
              className="pointer-events-auto absolute inset-y-2 right-0 w-2 touch-none cursor-grab active:cursor-grabbing"
              data-bestie-drag-edge="right"
              data-bestie-drag-handle
            />
          </div>
        ) : null}
      </motion.div>
    </div>
  );
}

function Trigger({
  ariaLabel,
  children,
  className,
}: {
  ariaLabel: string;
  children: React.ReactNode;
  className?: string;
}) {
  const { anchor, direction, open, setOpen, triggerRef } = useBloomContext();
  if (open) return null;

  return (
    <motion.div
      aria-expanded={open}
      aria-haspopup="menu"
      aria-label={ariaLabel}
      className={className}
      key="bloom-trigger"
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        setOpen(true);
      }}
      onKeyDown={(event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        setOpen(true);
      }}
      ref={triggerRef}
      role="button"
      style={{
        ...(anchor === "end" ? { right: 0 } : { left: 0 }),
        ...(direction === "top" ? { bottom: 0 } : { top: 0 }),
        position: "absolute",
      }}
      tabIndex={0}
    >
      {children}
    </motion.div>
  );
}

function Content({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  const { anchor, contentRef, direction, open } = useBloomContext();
  const reduceMotion = useReducedMotion();
  const hiddenY = direction === "top" ? 8 : -8;

  return (
    <AnimatePresence>
      {open ? (
        <motion.div
          animate={{
            filter: "blur(0px)",
            opacity: 1,
            transform: "translate3d(0, 0, 0) scale(1)",
          }}
          className={className}
          data-testid="bestie-bloom-content"
          exit={{
            filter: reduceMotion ? "blur(0px)" : "blur(8px)",
            opacity: 0,
            transform: reduceMotion
              ? "translate3d(0, 0, 0) scale(1)"
              : `translate3d(0, ${direction === "top" ? 24 : -24}px, 0) scale(0.95)`,
          }}
          initial={{
            filter: reduceMotion ? "blur(0px)" : "blur(10px)",
            opacity: 0,
            transform: reduceMotion
              ? "translate3d(0, 0, 0) scale(1)"
              : `translate3d(0, ${hiddenY}px, 0) scale(0.95)`,
          }}
          key="bloom-content"
          ref={contentRef}
          role="menu"
          style={{
            transformOrigin: `${
              anchor === "start"
                ? "left"
                : anchor === "end"
                  ? "right"
                  : "center"
            } ${direction === "top" ? "bottom" : "top"}`,
          }}
          transition={
            reduceMotion
              ? { duration: 0.12 }
              : {
                  bounce: 0.15,
                  delay: 0.03,
                  type: "spring",
                  visualDuration: 0.2,
                }
          }
        >
          {children}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}

export const Bloom = { Container, Content, Root, Trigger };
