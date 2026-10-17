package com.moodflow.ime.logic

import com.moodflow.ime.logic.FieldPolicy.Companion.IME_FLAG_NO_PERSONALIZED_LEARNING
import com.moodflow.ime.logic.FieldPolicy.Companion.TYPE_CLASS_NUMBER
import com.moodflow.ime.logic.FieldPolicy.Companion.TYPE_CLASS_PHONE
import com.moodflow.ime.logic.FieldPolicy.Companion.TYPE_CLASS_TEXT
import com.moodflow.ime.logic.FieldPolicy.Companion.TYPE_NUMBER_VARIATION_PASSWORD
import com.moodflow.ime.logic.FieldPolicy.Companion.TYPE_TEXT_VARIATION_PASSWORD
import com.moodflow.ime.logic.FieldPolicy.Companion.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
import com.moodflow.ime.logic.FieldPolicy.Companion.TYPE_TEXT_VARIATION_WEB_PASSWORD
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FieldPolicyTest {
    // TYPE_TEXT_VARIATION_EMAIL_ADDRESS：普通文本变体
    private val email = TYPE_CLASS_TEXT or 0x20

    @Test
    fun plainTextUsesEngineAndAllowsEverything() {
        val p = FieldPolicy.of(TYPE_CLASS_TEXT, 0)
        assertFalse(p.isPrivate)
        assertTrue(p.useEngine && p.allowBuffer && p.allowClipboardCapture)
        assertFalse(FieldPolicy.of(email, 0).isPrivate)
    }

    @Test
    fun everyPasswordVariationIsPrivate() {
        for (v in listOf(
            TYPE_TEXT_VARIATION_PASSWORD,
            TYPE_TEXT_VARIATION_VISIBLE_PASSWORD,
            TYPE_TEXT_VARIATION_WEB_PASSWORD,
        )) {
            val p = FieldPolicy.of(TYPE_CLASS_TEXT or v, 0)
            assertTrue("variation $v", p.password)
            assertFalse(p.useEngine)
            assertFalse(p.allowBuffer)
            assertFalse(p.allowClipboardCapture)
        }
        assertTrue(FieldPolicy.of(TYPE_CLASS_NUMBER or TYPE_NUMBER_VARIATION_PASSWORD, 0).password)
    }

    @Test
    fun noPersonalizedLearningIsPrivate() {
        val p = FieldPolicy.of(TYPE_CLASS_TEXT, IME_FLAG_NO_PERSONALIZED_LEARNING)
        assertFalse(p.password)
        assertTrue(p.isPrivate)
        assertFalse(p.useEngine)
        assertFalse(p.allowBuffer)
        assertFalse(p.allowClipboardCapture)
    }

    @Test
    fun numericFieldsSkipEngineButAreNotPrivate() {
        val p = FieldPolicy.of(TYPE_CLASS_PHONE, 0)
        assertFalse(p.isPrivate)
        assertFalse(p.useEngine)
        assertTrue(p.prefersNumberPad)
        // 数字类别下 0x80 不是密码变体
        assertFalse(FieldPolicy.of(TYPE_CLASS_NUMBER or 0x80, 0).password)
    }

    @Test
    fun nullInputTypeIsTreatedAsText() {
        assertTrue(FieldPolicy.of(0, 0).useEngine)
    }

    @Test
    fun clipboardGate() {
        val normal = FieldPolicy.of(TYPE_CLASS_TEXT, 0)
        val pw = FieldPolicy.of(TYPE_CLASS_TEXT or TYPE_TEXT_VARIATION_PASSWORD, 0)
        assertTrue(ClipboardGate.shouldCapture(true, normal))
        assertFalse("键盘不在前台不采集", ClipboardGate.shouldCapture(false, normal))
        assertFalse(ClipboardGate.shouldCapture(true, pw))
        assertFalse(ClipboardGate.shouldCapture(true, FieldPolicy.NONE))
        assertTrue(ClipboardGate.sensitive(true, normal))
        assertTrue(ClipboardGate.sensitive(false, pw))
        assertFalse(ClipboardGate.sensitive(false, normal))
    }
}

class FieldTrackerTest {
    private val a = FieldKey("com.example", 1, TYPE_CLASS_TEXT)
    private val b = FieldKey("com.example", 2, TYPE_CLASS_TEXT)

    @Test
    fun differentFieldIsNew() {
        val t = FieldTracker()
        assertEquals(FieldChange.NEW_FIELD, t.onStartInput(a, restarting = false))
        assertEquals(FieldChange.NEW_FIELD, t.onStartInput(b, restarting = true))
    }

    @Test
    fun restartOfSameFieldBumpsRevisionOnly() {
        val t = FieldTracker()
        t.onStartInput(a, restarting = false)
        val first = t.current!!
        assertEquals(FieldChange.SAME_FIELD_RESTART, t.onStartInput(a, restarting = true))
        val second = t.current!!
        assertEquals(first.id, second.id)
        assertNotEquals(first.revision, second.revision)
    }

    @Test
    fun comingBackAfterFinishIsNew() {
        val t = FieldTracker()
        t.onStartInput(a, restarting = false)
        t.onFinishInput()
        assertNull(t.current)
        assertEquals(FieldChange.NEW_FIELD, t.onStartInput(a, restarting = true))
    }

    @Test
    fun generationInvalidatesStaleEvents() {
        val t = FieldTracker()
        t.onStartInput(a, restarting = false)
        val captured = t.generation
        assertTrue(t.isCurrent(captured))
        t.onFinishInput()
        t.onStartInput(b, restarting = false)
        assertFalse("换输入框后迟到的点击作废", t.isCurrent(captured))
    }

    @Test
    fun stableIdIsDeterministicAndPositive() {
        assertEquals(FieldTracker.stableId(a), FieldTracker.stableId(a.copy()))
        assertNotEquals(FieldTracker.stableId(a), FieldTracker.stableId(b))
        assertTrue(FieldTracker.stableId(a) >= 0)
    }
}
