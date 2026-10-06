// Canna Auto-Hop 0.1.0 preview. MIT license; see LICENSE.
// Run on the host after loading a local map: script_execute canna_autohop
// Jump input is IN_JUMP, shared by keyboard, remapped keys and controllers.
if ("CannaAutoHop" in getroottable()) {
    if (::CannaAutoHop.ticker != null && ::CannaAutoHop.ticker.IsValid())
        ::CannaAutoHop.ticker.Kill();
}

::CannaAutoHop <- {
    enabled = true,
    jumpSpeed = 268.0,
    trace = false,
    lastHop = {},
    ticker = null,

    function LocalHost() {
        return !IsDedicatedServer() && GetListenServerHost() != null;
    },

    function Tick() {
        if (!enabled || !LocalHost()) return 0.01;
        local player = null;
        local present = {};
        while ((player = Entities.FindByClassname(player, "player")) != null) {
            local id = player.GetEntityIndex();
            present[id] <- true;
            if (IsPlayerABot(player) || !player.IsSurvivor() || player.IsDead()
                || player.IsIncapacitated() || player.IsHangingFromLedge()) continue;
            if ((player.GetButtonMask() & 2) == 0) continue;
            // Only ordinary walking, on the ground, outside deep water.
            if (!NetProps.HasProp(player, "movetype")
                || NetProps.GetPropInt(player, "movetype") != 2) continue;
            if (NetProps.GetPropInt(player, "m_nWaterLevel") >= 2) continue;
            local flags = NetProps.GetPropInt(player, "m_fFlags");
            if ((flags & 1) == 0) continue;
            local pinned = false;
            foreach (prop in ["m_tongueOwner", "m_pounceAttacker", "m_jockeyAttacker", "m_carryAttacker", "m_pummelAttacker"]) {
                if (NetProps.HasProp(player, prop) && NetProps.GetPropEntity(player, prop) != null)
                    pinned = true;
            }
            if (pinned) continue;
            local now = Time();
            if (id in lastHop && now - lastHop[id] < 0.08) continue;
            local velocity = player.GetVelocity();
            // One vertical pulse per landing; horizontal momentum is unchanged.
            player.SetVelocity(Vector(velocity.x, velocity.y, jumpSpeed));
            NetProps.SetPropInt(player, "m_fFlags", flags & ~1);
            if (NetProps.HasProp(player, "m_hGroundEntity"))
                NetProps.SetPropEntity(player, "m_hGroundEntity", null);
            lastHop[id] <- now;
            if (trace) printl("[Canna Auto-Hop test] hop");
        }
        local departed = [];
        foreach (id, value in lastHop) if (!(id in present)) departed.append(id);
        foreach (id in departed) delete lastHop[id];
        return 0.01;
    },

    function Start() {
        if (!LocalHost()) {
            printl("[Canna Auto-Hop] Start a local listen-server map first.");
            return;
        }
        ticker = SpawnEntityFromTable("logic_script", { targetname = "canna_autohop_tick" });
        if (ticker == null || !ticker.ValidateScriptScope()) {
            printl("[Canna Auto-Hop] Could not create the tick handler.");
            return;
        }
        ticker.GetScriptScope().CannaAutoHopThink <- function() { return ::CannaAutoHop.Tick(); };
        AddThinkToEnt(ticker, "CannaAutoHopThink");
        printl("[Canna Auto-Hop preview] Hold your bound jump button. Enabled for human survivors on this local server.");
    }
};
::CannaAutoHop.Start();

