package com.moodflow.ime.logic

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** 与 Rust 侧 `wire.rs` 的编码测试成对：两边各自锁住同一份格式。 */
class WireTest {
    private val us = Wire.US

    @Test
    fun decodesOutcomeInOrder() {
        val o = Wire.outcome(
            arrayOf("cm", "C你好", "S6${us}ni'hao", "D2", "R1$us.", "M-1"),
        )
        assertTrue(o.consumed)
        assertTrue(o.modeChanged)
        assertEquals(
            listOf(
                EditOp.Commit("你好"),
                EditOp.SetComposition("ni'hao", 6),
                EditOp.DeleteBackward(2),
                EditOp.ReplaceBackward(1, "."),
                EditOp.MoveCursor(-1),
            ),
            o.ops,
        )
    }

    @Test
    fun passthroughAndNullAreNotConsumed() {
        assertEquals(KeyOutcome.PASSTHROUGH, Wire.outcome(arrayOf("")))
        assertEquals(KeyOutcome.PASSTHROUGH, Wire.outcome(null))
        assertFalse(Wire.outcome(arrayOf("m")).consumed)
    }

    @Test
    fun separatorInsideTextDoesNotShiftFields() {
        val o = Wire.outcome(arrayOf("c", "S1${us}a${us}b"))
        assertEquals(listOf(EditOp.SetComposition("a${us}b", 1)), o.ops)
        val clip = Wire.clipEntries(arrayOf("7${us}t${us}1${us}x${us}y"))
        assertEquals(ClipEntry(7, isLink = false, pinned = true, text = "x${us}y"), clip.single())
    }

    @Test
    fun unknownOpsAreSkipped() {
        val o = Wire.outcome(arrayOf("c", "Zwhatever", "Cok", "Dnotanumber"))
        assertEquals(listOf(EditOp.Commit("ok")), o.ops)
    }

    @Test
    fun decodesCandidateWindow() {
        val w = Wire.candidateWindow(arrayOf("10${us}42${us}11", "你", "尼"))
        assertEquals(CandidateWindow(10, 42, 11, listOf("你", "尼")), w)
        assertEquals(CandidateWindow.EMPTY, Wire.candidateWindow(emptyArray()))
    }

    @Test
    fun decodesSchemasAndRegistry() {
        val s = Wire.schemas(arrayOf("pinyin${us}全拼${us}拼${us}1", "wubi86${us}五笔${us}五${us}0"))
        assertEquals(SchemaEntry("pinyin", "全拼", "拼", true), s[0])
        assertFalse(s[1].active)
        val r = Wire.configRegistry(arrayOf("ui.layout${us}enum${us}a\u001Eb", "x${us}bool$us"))
        assertEquals(listOf("a", "b"), r[0].options)
        assertTrue(r[1].options.isEmpty())
    }

    @Test
    fun decodesChord() {
        assertNull(Wire.chord(null))
        assertEquals(ChordResult.NoMatch, Wire.chord("N"))
        assertEquals(ChordResult.Emit("zhang", fragment = false), Wire.chord("Eszhang"))
        assertEquals(ChordResult.Emit("zh", fragment = true), Wire.chord("Efzh"))
    }

    @Test
    fun fieldsPadsMissing() {
        assertEquals(listOf("a", "", ""), Wire.fields("a", 3))
    }
}
